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
use renegade_common::types::token::Token;

use super::Server;
use crate::server::helpers::{generate_quote_uuid, get_nominal_buy_token_price};
use crate::telemetry::labels::{
    GAS_SPONSORSHIP_VALUE, L1_COST_PER_BYTE_TAG, L2_BASE_FEE_TAG, REFUND_AMOUNT_TAG,
    REFUND_ASSET_TAG, REMAINING_TIME_TAG, REMAINING_VALUE_TAG, REQUEST_ID_METRIC_TAG,
};
use crate::{error::AuthServerError, server::helpers::ethers_u256_to_bigdecimal};

pub mod contract_interaction;
mod refund_calculation;

// -------------
// | Constants |
// -------------

/// The ticker for native ETH
const ETH_TICKER: &str = "ETH";

/// The ticker for WETH
const WETH_TICKER: &str = "WETH";

/// The error message emitted when a refund asset ticker cannot be found
const REFUND_ASSET_TICKER_ERROR_MSG: &str = "failed to get refund asset ticker";

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
        let nominal_price = if gas_sponsorship_info.refund_native_eth {
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

        let nominal_amount = BigDecimal::from_u128(gas_sponsorship_info.refund_amount)
            .expect("u128 should be representable as BigDecimal");

        let value_bigdecimal = nominal_amount * nominal_price;

        let value = value_bigdecimal.to_f64().ok_or(AuthServerError::gas_sponsorship(
            "failed to convert gas sponsorship value to f64",
        ))?;

        self.rate_limiter.record_gas_sponsorship(key.clone(), value).await;

        self.record_gas_sponsorship_metrics(
            value,
            gas_sponsorship_info,
            match_bundle,
            key,
            request_id,
        )
        .await?;

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

    // -----------
    // | Helpers |
    // -----------

    /// Record the dollar value of sponsored gas for a settled match
    async fn record_gas_sponsorship_metrics(
        &self,
        gas_sponsorship_value: f64,
        gas_sponsorship_info: GasSponsorshipInfo,
        match_bundle: &AtomicMatchApiBundle,
        key: String,
        request_id: String,
    ) -> Result<(), AuthServerError> {
        // Extra sponsorship metadata:
        // - Remaining value in user's rate limit bucket
        // - Remaining time in user's rate limit bucket
        // - Refund asset
        // - Refund amount (whole units)
        // - Gas prices (L1 & L2)

        let (remaining_value, remaining_time) =
            self.rate_limiter.remaining_gas_sponsorship_value_and_time(key).await;

        let (refund_asset_ticker, refund_amount_whole) = if gas_sponsorship_info.refund_native_eth {
            // WETH uses the same decimals as ETH, so we use it to obtain the refund amount
            // in whole units
            let weth = Token::from_ticker(WETH_TICKER);
            let refund_amount_whole = weth.convert_to_decimal(gas_sponsorship_info.refund_amount);
            (ETH_TICKER.to_string(), refund_amount_whole)
        } else {
            let refund_asset = Token::from_addr(&match_bundle.receive.mint);
            let refund_amount_whole =
                refund_asset.convert_to_decimal(gas_sponsorship_info.refund_amount);

            let refund_asset_ticker = refund_asset
                .get_ticker()
                .ok_or(AuthServerError::gas_cost_sampler(REFUND_ASSET_TICKER_ERROR_MSG))?;

            (refund_asset_ticker, refund_amount_whole)
        };

        let (_, l2_base_fee, l1_cost_per_byte) = self
            .gas_cost_sampler
            .sample_gas_prices()
            .await
            .map_err(AuthServerError::gas_sponsorship)?;

        let labels = vec![
            (REQUEST_ID_METRIC_TAG.to_string(), request_id),
            (REMAINING_VALUE_TAG.to_string(), remaining_value.to_string()),
            (REMAINING_TIME_TAG.to_string(), remaining_time.as_secs().to_string()),
            (REFUND_ASSET_TAG.to_string(), refund_asset_ticker),
            (REFUND_AMOUNT_TAG.to_string(), refund_amount_whole.to_string()),
            (L2_BASE_FEE_TAG.to_string(), l2_base_fee.to_string()),
            (L1_COST_PER_BYTE_TAG.to_string(), l1_cost_per_byte.to_string()),
        ];

        metrics::gauge!(GAS_SPONSORSHIP_VALUE, &labels).set(gas_sponsorship_value);

        Ok(())
    }
}
