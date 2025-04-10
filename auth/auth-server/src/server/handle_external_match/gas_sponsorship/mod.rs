//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use auth_server_api::{
    GasSponsorshipInfo, GasSponsorshipQueryParams, SignedGasSponsorshipInfo,
    SponsoredMatchResponse, SponsoredQuoteResponse,
};
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use ethers::utils::WEI_IN_ETHER;

use refund_calculation::{apply_gas_sponsorship_to_match_bundle, apply_gas_sponsorship_to_quote};
use renegade_api::http::external_match::{
    AtomicMatchApiBundle, ExternalMatchResponse, ExternalOrder, ExternalQuoteResponse,
};
use renegade_common::types::token::Token;

use super::Server;
use crate::server::helpers::{generate_quote_uuid, get_nominal_buy_token_price};
use crate::telemetry::labels::{
    GAS_SPONSORSHIP_VALUE, KEY_DESCRIPTION_METRIC_TAG, L1_COST_PER_BYTE_TAG, L2_BASE_FEE_TAG,
    REFUND_AMOUNT_TAG, REFUND_ASSET_TAG, REMAINING_TIME_TAG, REMAINING_VALUE_TAG,
    REQUEST_ID_METRIC_TAG, SDK_VERSION_METRIC_TAG,
};
use crate::{error::AuthServerError, server::helpers::ethers_u256_to_bigdecimal};

pub mod contract_interaction;
pub mod refund_calculation;

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
    /// Generate gas sponsorship info for a given user's order, if permissible
    /// according to the rate limit and query params
    pub(crate) async fn maybe_generate_gas_sponsorship_info(
        &self,
        key_desc: String,
        order: &ExternalOrder,
        query_params: &GasSponsorshipQueryParams,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        // Parse query params
        let (sponsorship_disabled, refund_address, refund_native_eth) =
            query_params.get_or_default();

        // Check gas sponsorship rate limit
        let gas_sponsorship_rate_limited = !self.check_gas_sponsorship_rate_limit(key_desc).await;

        // TODO: Check if order size is above configured sponsored order size minimum
        let sponsor_match = !(gas_sponsorship_rate_limited || sponsorship_disabled);

        if !sponsor_match {
            return Ok(None);
        }

        let refund_amount = self.compute_refund_amount_for_order(order, refund_native_eth).await?;

        GasSponsorshipInfo::new(refund_amount, refund_native_eth, refund_address)
            .map(Some)
            .map_err(AuthServerError::gas_sponsorship)
    }

    /// Construct a sponsored match response from an external match response
    pub(crate) fn construct_sponsored_match_response(
        &self,
        mut external_match_resp: ExternalMatchResponse,
        gas_sponsorship_info: GasSponsorshipInfo,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        let refund_native_eth = gas_sponsorship_info.refund_native_eth;
        let refund_address = gas_sponsorship_info.get_refund_address();
        let refund_amount = gas_sponsorship_info.get_refund_amount();

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
            is_sponsored: true,
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
        gas_sponsorship_info: &GasSponsorshipInfo,
        key: String,
        request_id: String,
        sdk_version: String,
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
            sdk_version,
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
        gas_sponsorship_info: &GasSponsorshipInfo,
        match_bundle: &AtomicMatchApiBundle,
        key: String,
        request_id: String,
        sdk_version: String,
    ) -> Result<(), AuthServerError> {
        // Extra sponsorship metadata:
        // - Remaining value in user's rate limit bucket
        // - Remaining time in user's rate limit bucket
        // - Refund asset
        // - Refund amount (whole units)
        // - Gas prices (L1 & L2)

        let (remaining_value, remaining_time) =
            self.rate_limiter.remaining_gas_sponsorship_value_and_time(key.clone()).await;

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
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id),
            (SDK_VERSION_METRIC_TAG.to_string(), sdk_version),
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
