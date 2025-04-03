//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::{Address as AlloyAddress, U256};
use auth_server_api::{
    GasSponsorshipInfo, SignedGasSponsorshipInfo, SponsoredMatchResponse, SponsoredQuoteResponse,
};
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use ethers::utils::WEI_IN_ETHER;

use refund_calculation::{
    update_match_bundle_with_gas_sponsorship, update_quote_with_gas_sponsorship,
};
use renegade_api::http::external_match::{
    AtomicMatchApiBundle, ExternalMatchResponse, ExternalQuoteResponse,
};

use super::Server;
use crate::server::helpers::{generate_quote_uuid, get_nominal_buy_token_price};
use crate::telemetry::helpers::record_gas_sponsorship_metrics;
use crate::{error::AuthServerError, server::helpers::ethers_u256_to_bigdecimal};

pub mod contract_interaction;
mod refund_calculation;

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Construct a sponsored match response from an external match response
    pub(crate) fn construct_sponsored_match_response(
        &self,
        mut external_match_resp: ExternalMatchResponse,
        refund_native_eth: bool,
        refund_address: AlloyAddress,
        refund_amount: U256,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        let gas_sponsor_calldata = self
            .generate_gas_sponsor_calldata(
                &external_match_resp,
                refund_address,
                refund_native_eth,
                refund_amount,
            )?
            .into();

        external_match_resp.match_bundle.settlement_tx.set_to(self.gas_sponsor_address);
        external_match_resp.match_bundle.settlement_tx.set_data(gas_sponsor_calldata);

        let gas_sponsorship_info =
            GasSponsorshipInfo::new(refund_amount, refund_native_eth, refund_address)
                .map_err(AuthServerError::gas_sponsorship)?;

        if gas_sponsorship_info.requires_match_result_update() {
            // The `ExternalMatchResponse` from the relayer doesn't account for gas
            // sponsorship, so we need to update the match bundle to reflect the
            // refund.
            update_match_bundle_with_gas_sponsorship(
                &mut external_match_resp.match_bundle,
                gas_sponsorship_info.refund_amount,
            );
        }

        Ok(SponsoredMatchResponse {
            match_bundle: external_match_resp.match_bundle,
            is_sponsored: true,
            gas_sponsorship_info: Some(gas_sponsorship_info),
        })
    }

    /// Construct a sponsored quote response from an external quote response
    pub(crate) async fn construct_sponsored_quote_response(
        &self,
        mut external_quote_response: ExternalQuoteResponse,
        refund_native_eth: bool,
        refund_address: AlloyAddress,
    ) -> Result<SponsoredQuoteResponse, AuthServerError> {
        // Compute refund amount
        let refund_amount = self
            .get_refund_amount(
                &external_quote_response.signed_quote.match_result(),
                refund_native_eth,
            )
            .await?;

        let gas_sponsorship_info =
            GasSponsorshipInfo::new(refund_amount, refund_native_eth, refund_address)
                .map_err(AuthServerError::gas_sponsorship)?;

        if gas_sponsorship_info.requires_match_result_update() {
            // Update quote to reflect sponsorship
            update_quote_with_gas_sponsorship(
                &mut external_quote_response.signed_quote.quote,
                gas_sponsorship_info.refund_amount,
            )?;
        }

        // Since we cache gas sponsorship info in Redis, we don't need to sign it.
        // The `SignedGasSponsorshipInfo` struct is only used for backwards
        // compatibility
        #[allow(deprecated)]
        let signed_gas_sponsorship_info =
            SignedGasSponsorshipInfo { gas_sponsorship_info, signature: String::new() };

        Ok(SponsoredQuoteResponse {
            signed_quote: external_quote_response.signed_quote,
            gas_sponsorship_info: Some(signed_gas_sponsorship_info),
        })
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        gas_sponsorship_info: GasSponsorshipInfo,
        key: String,
        request_id: String,
    ) -> Result<(), AuthServerError> {
        let GasSponsorshipInfo { refund_amount, refund_native_eth, .. } = gas_sponsorship_info;

        let nominal_price = if refund_native_eth {
            let price_f64: f64 = self
                .price_reporter_client
                .get_eth_price()
                .await
                .map_err(AuthServerError::gas_sponsorship)?;

            let price_bigdecimal = BigDecimal::from_f64(price_f64).ok_or(
                AuthServerError::gas_sponsorship("failed to convert ETH price to BigDecimal"),
            )?;

            let adjustment = ethers_u256_to_bigdecimal(WEI_IN_ETHER);

            price_bigdecimal / adjustment
        } else {
            // If we did not refund via native ETH, it must have been through the buy-side
            // token. Thus we compute the nominal price of the buy-side
            // token from the match result.
            get_nominal_buy_token_price(&match_bundle.receive.mint, &match_bundle.match_result)?
        };

        let nominal_amount = BigDecimal::from_u128(refund_amount)
            .expect("u128 should be representable as BigDecimal");

        let value_bigdecimal = nominal_amount * nominal_price;

        let value = value_bigdecimal.to_f64().ok_or(AuthServerError::gas_sponsorship(
            "failed to convert gas sponsorship value to f64",
        ))?;

        self.rate_limiter.record_gas_sponsorship(key, value).await;

        record_gas_sponsorship_metrics(value, request_id);

        Ok(())
    }

    /// Cache the gas sponsorship info for a given quote in Redis
    /// if it exists
    pub async fn cache_quote_gas_sponsorship_info(
        &self,
        quote_res: &SponsoredQuoteResponse,
    ) -> Result<(), AuthServerError> {
        if quote_res.gas_sponsorship_info.is_none() {
            return Ok(());
        }

        let redis_key = generate_quote_uuid(&quote_res.signed_quote);
        let gas_sponsorship_info =
            &quote_res.gas_sponsorship_info.as_ref().unwrap().gas_sponsorship_info;

        self.write_gas_sponsorship_info_to_redis(redis_key, gas_sponsorship_info).await
    }
}
