//! Helpers for executing subroutines in the on-chain event listener
use auth_server_api::GasSponsorshipInfo;
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

use crate::{chain_events::listener::OnChainEventListenerExecutor, store::BundleContext};
use crate::{
    error::AuthServerError,
    telemetry::{
        helpers::{extend_labels_with_base_asset, record_volume_with_tags},
        labels::{
            EXTERNAL_MATCH_SETTLED_BASE_VOLUME, EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
            GAS_SPONSORED_METRIC_TAG, GAS_SPONSORSHIP_VALUE, KEY_DESCRIPTION_METRIC_TAG,
            L1_COST_PER_BYTE_TAG, L2_BASE_FEE_TAG, REFUND_AMOUNT_TAG, REFUND_ASSET_TAG,
            REMAINING_TIME_TAG, REMAINING_VALUE_TAG, REQUEST_ID_METRIC_TAG, SDK_VERSION_METRIC_TAG,
            SETTLEMENT_STATUS_TAG,
        },
    },
};

// -------------
// | Constants |
// -------------

/// The ticker for native ETH
const ETH_TICKER: &str = "ETH";

/// The ticker for WETH
const WETH_TICKER: &str = "WETH";

/// The error message emitted when a refund asset ticker cannot be found
const REFUND_ASSET_TICKER_ERROR_MSG: &str = "failed to get refund asset ticker";

impl OnChainEventListenerExecutor {
    /// Record settlement metrics for a bundle
    pub fn record_settlement_metrics(
        &self,
        ctx: &BundleContext,
        match_result: &ApiExternalMatchResult,
    ) {
        let labels = self.get_labels(ctx);
        record_volume_with_tags(
            &match_result.base_mint,
            match_result.base_amount,
            EXTERNAL_MATCH_SETTLED_BASE_VOLUME,
            &labels,
        );

        let labels = extend_labels_with_base_asset(&match_result.base_mint, labels);
        record_volume_with_tags(
            &match_result.quote_mint,
            match_result.quote_amount,
            EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
            &labels,
        );
    }

    /// Increment the token balance for a given API user
    pub async fn add_bundle_rate_limit_token(&self, key_description: String, shared: bool) {
        self.rate_limiter.add_bundle_token(key_description, shared).await;
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        ctx: &BundleContext,
        match_result: &ApiExternalMatchResult,
        gas_sponsorship_info: &GasSponsorshipInfo,
    ) -> Result<(), AuthServerError> {
        let refund_asset = if gas_sponsorship_info.refund_native_eth {
            Token::from_ticker(WETH_TICKER)
        } else {
            match match_result.direction {
                OrderSide::Buy => Token::from_addr(&match_result.base_mint),
                OrderSide::Sell => Token::from_addr(&match_result.quote_mint),
            }
        };
        let nominal_price =
            self.price_reporter_client.get_nominal_price(&refund_asset.get_addr()).await?;

        let nominal_amount = BigDecimal::from_u128(gas_sponsorship_info.refund_amount)
            .expect("u128 should be representable as BigDecimal");

        let value_bigdecimal = nominal_amount * nominal_price;

        let value = value_bigdecimal.to_f64().ok_or(AuthServerError::gas_sponsorship(
            "failed to convert gas sponsorship value to f64",
        ))?;

        self.rate_limiter.record_gas_sponsorship(ctx.key_description.clone(), value).await;

        self.record_gas_sponsorship_metrics(
            value,
            gas_sponsorship_info,
            match_result,
            ctx.key_description.clone(),
            ctx.request_id.clone(),
            ctx.sdk_version.clone(),
        )
        .await?;

        Ok(())
    }

    /// Record the dollar value of sponsored gas for a settled match
    async fn record_gas_sponsorship_metrics(
        &self,
        gas_sponsorship_value: f64,
        gas_sponsorship_info: &GasSponsorshipInfo,
        match_result: &ApiExternalMatchResult,
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
            let refund_asset = match match_result.direction {
                OrderSide::Buy => Token::from_addr(&match_result.base_mint),
                OrderSide::Sell => Token::from_addr(&match_result.quote_mint),
            };
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

    /// Get the labels for a bundle
    fn get_labels(&self, ctx: &BundleContext) -> Vec<(String, String)> {
        vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.key_description.clone()),
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.clone()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), ctx.is_sponsored.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), ctx.sdk_version.clone()),
            (SETTLEMENT_STATUS_TAG.to_string(), "true".to_string()),
        ]
    }
}
