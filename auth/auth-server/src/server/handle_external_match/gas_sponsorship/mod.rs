//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::{Address as AlloyAddress, U256};
use auth_server_api::{
    GasSponsorshipInfo, SignedGasSponsorshipInfo, SponsoredMatchResponse, SponsoredQuoteResponse,
};
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use ethers::{types::Address, utils::WEI_IN_ETHER};

use renegade_api::http::external_match::{
    AtomicMatchApiBundle, ExternalMatchResponse, ExternalQuoteResponse,
};
use renegade_util::hex::{bytes_from_hex_string, bytes_to_hex_string};

use super::Server;
use crate::server::helpers::get_nominal_buy_token_price;
use crate::telemetry::helpers::record_gas_sponsorship_metrics;
use crate::{error::AuthServerError, server::helpers::ethers_u256_to_bigdecimal};

pub mod contract_interaction;
mod refund_calculation;

// -------------
// | Constants |
// -------------

/// The error message for an invalid gas sponsorship info signature
const ERR_INVALID_GAS_SPONSORSHIP_INFO_SIGNATURE: &str = "invalid gas sponsorship info signature";

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

        Ok(SponsoredMatchResponse {
            match_bundle: external_match_resp.match_bundle,
            is_sponsored: true,
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

        // Only update the quote if the refund is in-kind and the refund address is not
        // set
        if gas_sponsorship_info.requires_quote_update() {
            // Update quote to reflect sponsorship
            self.update_quote_with_sponsorship(
                &mut external_quote_response.signed_quote.quote,
                gas_sponsorship_info.refund_amount,
            )?;
        }

        let signed_gas_sponsorship_info = self.sign_gas_sponsorship_info(gas_sponsorship_info)?;

        Ok(SponsoredQuoteResponse {
            external_quote_response,
            gas_sponsorship_info: Some(signed_gas_sponsorship_info),
        })
    }

    /// Sign the given gas sponsorship info
    pub fn sign_gas_sponsorship_info(
        &self,
        gas_sponsorship_info: GasSponsorshipInfo,
    ) -> Result<SignedGasSponsorshipInfo, AuthServerError> {
        let gas_sponsorship_info_bytes =
            serde_json::to_vec(&gas_sponsorship_info).map_err(AuthServerError::serde)?;

        let signature = self.management_key.compute_mac(&gas_sponsorship_info_bytes);
        let signature_hex = bytes_to_hex_string(&signature);

        Ok(SignedGasSponsorshipInfo { gas_sponsorship_info, signature: signature_hex })
    }

    /// Validate the given signed gas sponsorship info
    pub fn validate_gas_sponsorship_info_signature(
        &self,
        signed_gas_sponsorship_info: &SignedGasSponsorshipInfo,
    ) -> Result<(), AuthServerError> {
        let gas_sponsorship_info_bytes =
            serde_json::to_vec(&signed_gas_sponsorship_info.gas_sponsorship_info)
                .map_err(AuthServerError::serde)?;

        let mac_bytes = bytes_from_hex_string(&signed_gas_sponsorship_info.signature)
            .map_err(AuthServerError::serde)?;

        if !self.management_key.verify_mac(&gas_sponsorship_info_bytes, &mac_bytes) {
            return Err(AuthServerError::gas_sponsorship(
                ERR_INVALID_GAS_SPONSORSHIP_INFO_SIGNATURE,
            ));
        }

        Ok(())
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        is_sponsored: bool,
        key: String,
        request_id: String,
    ) -> Result<(), AuthServerError> {
        if is_sponsored
            && let Some((token_addr, amount, tx_hash)) =
                self.get_refunded_amount_and_tx(&match_bundle.settlement_tx).await?
        {
            let nominal_price = if token_addr == Address::zero() {
                // The zero address indicates that the gas was sponsored via native ETH.
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

            let nominal_amount = ethers_u256_to_bigdecimal(amount);
            let value_bigdecimal = nominal_amount * nominal_price;

            let value = value_bigdecimal.to_f64().ok_or(AuthServerError::gas_sponsorship(
                "failed to convert gas sponsorship value to f64",
            ))?;

            self.rate_limiter.record_gas_sponsorship(key, value).await;

            record_gas_sponsorship_metrics(value, tx_hash, request_id);
        }

        Ok(())
    }
}
