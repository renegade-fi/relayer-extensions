//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::U256;
use auth_server_api::{
    GasSponsorshipInfo, GasSponsorshipQueryParams, SponsoredMatchResponse, SponsoredQuoteResponse,
};

use refund_calculation::{apply_gas_sponsorship_to_match_bundle, apply_gas_sponsorship_to_quote};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_external_api::http::external_match::{ExternalMatchResponse, ExternalQuoteResponse};
use renegade_external_api::types::ExternalOrder;
use renegade_types_core::Token;

use super::Server;
use crate::error::AuthServerError;
use crate::server::helpers::generate_quote_uuid;

pub mod contract_interaction;
pub mod refund_calculation;

// -------------
// | Constants |
// -------------

/// The ticker for WETH
const WETH_TICKER: &str = "WETH";

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Generate gas sponsorship info for a given user's order, if permissible
    /// according to the rate limit and query params
    pub(crate) async fn generate_sponsorship_info(
        &self,
        key_desc: &str,
        order: &ExternalOrder,
        query_params: &GasSponsorshipQueryParams,
    ) -> Result<GasSponsorshipInfo, AuthServerError> {
        // Parse query params
        let (sponsorship_disabled, refund_address, refund_native_eth) =
            query_params.get_or_default();

        // Check gas sponsorship rate limit
        let rate_limited = !self.check_gas_sponsorship_rate_limit(key_desc).await?;

        let expected_quote_amount =
            self.get_quote_amount(order, FixedPoint::zero() /* relayer_fee */).await?;

        let expected_quote_amount_f64 = Token::usdc().convert_to_decimal(expected_quote_amount);
        let order_too_small = expected_quote_amount_f64 < self.min_sponsored_order_quote_amount;

        let sponsor_match = !(rate_limited || sponsorship_disabled || order_too_small);
        if !sponsor_match {
            return Ok(GasSponsorshipInfo::zero());
        }

        let refund_amount = self.compute_refund_amount_for_order(order, refund_native_eth).await?;
        GasSponsorshipInfo::new(refund_amount, refund_native_eth, refund_address)
            .map_err(AuthServerError::gas_sponsorship)
    }

    /// Construct a sponsored match response from an external match response
    pub(crate) fn construct_sponsored_match_response(
        &self,
        mut external_match_resp: ExternalMatchResponse,
        gas_sponsorship_info: GasSponsorshipInfo,
        sponsorship_nonce: U256,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        // Generate calldata for the gas sponsorship contract
        let gas_sponsor_calldata = self.generate_gas_sponsor_calldata(
            &external_match_resp,
            &gas_sponsorship_info,
            sponsorship_nonce,
        )?;

        let mut tx = external_match_resp.match_bundle.settlement_tx;
        tx = tx.to(self.gas_sponsor_address);
        tx.input.input = Some(gas_sponsor_calldata);
        external_match_resp.match_bundle.settlement_tx = tx;

        // The `ExternalMatchResponse` from the relayer doesn't account for gas
        // sponsorship, so we need to update the match bundle to reflect the
        // refund.
        if gas_sponsorship_info.requires_match_result_update() {
            apply_gas_sponsorship_to_match_bundle(
                &mut external_match_resp.match_bundle,
                gas_sponsorship_info.refund_amount,
            );
        }

        Ok(SponsoredMatchResponse {
            match_bundle: external_match_resp.match_bundle,
            gas_sponsorship_info: Some(gas_sponsorship_info),
        })
    }

    /// Construct a sponsored quote response from an external quote response
    pub(crate) fn construct_sponsored_quote_response(
        &self,
        mut external_quote_response: ExternalQuoteResponse,
        gas_sponsorship_info: GasSponsorshipInfo,
    ) -> Result<SponsoredQuoteResponse, AuthServerError> {
        let quote = &mut external_quote_response.signed_quote.quote;

        // Update quote price / receive amount to reflect sponsorship
        if gas_sponsorship_info.requires_match_result_update() {
            apply_gas_sponsorship_to_quote(quote, &gas_sponsorship_info)?;
        }

        Ok(SponsoredQuoteResponse {
            signed_quote: external_quote_response.signed_quote,
            gas_sponsorship_info: Some(gas_sponsorship_info),
        })
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
        let gas_sponsorship_info = quote_res.gas_sponsorship_info.as_ref().unwrap();

        self.write_gas_sponsorship_info_to_redis(redis_key, gas_sponsorship_info).await
    }
}
