//! Helpers for executing subroutines in the on-chain event listener
use alloy::providers::Provider;
use alloy::rpc::types::TransactionReceipt;
use alloy_primitives::U256;
use alloy_sol_types::SolEvent;
use auth_server_api::GasSponsorshipInfo;
use bigdecimal::{BigDecimal, ToPrimitive};
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

use crate::chain_events::abis::GasSponsorContract::SponsoredExternalMatch;
use crate::chain_events::utils::GPv2Settlement;
use crate::telemetry::labels::EXTERNAL_MATCH_SPREAD_COST;
use crate::{bundle_store::BundleContext, chain_events::listener::OnChainEventListenerExecutor};
use crate::{
    error::AuthServerError,
    telemetry::{
        helpers::{
            extend_labels_with_base_asset, extend_labels_with_side, record_volume_with_tags,
        },
        labels::{
            ASSET_METRIC_TAG, EXTERNAL_MATCH_ASSEMBLY_DELAY,
            EXTERNAL_MATCH_ASSEMBLY_TO_SETTLEMENT_DELAY, EXTERNAL_MATCH_SETTLED_BASE_VOLUME,
            EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME, EXTERNAL_MATCH_SETTLEMENT_DELAY,
            GAS_SPONSORED_METRIC_TAG, GAS_SPONSORSHIP_VALUE, KEY_DESCRIPTION_METRIC_TAG,
            L1_COST_PER_BYTE_TAG, L2_BASE_FEE_TAG, REFUND_AMOUNT_TAG, REFUND_ASSET_TAG,
            REQUEST_ID_METRIC_TAG, SDK_VERSION_METRIC_TAG, SETTLED_VIA_COWSWAP_TAG,
            SETTLEMENT_STATUS_TAG, SIDE_TAG,
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
        receipt: &TransactionReceipt,
        ctx: &BundleContext,
        match_result: &ApiExternalMatchResult,
    ) -> Result<(), AuthServerError> {
        let mut labels = self.get_labels(ctx);
        let is_settled_via_cowswap = self.detect_cowswap_settlement(receipt);
        labels.push((SETTLED_VIA_COWSWAP_TAG.to_string(), is_settled_via_cowswap.to_string()));

        record_volume_with_tags(
            &match_result.base_mint,
            match_result.base_amount,
            EXTERNAL_MATCH_SETTLED_BASE_VOLUME,
            &labels,
        );

        labels = extend_labels_with_base_asset(&match_result.base_mint, labels);
        labels = extend_labels_with_side(&match_result.direction, labels);
        record_volume_with_tags(
            &match_result.quote_mint,
            match_result.quote_amount,
            EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
            &labels,
        );

        Ok(())
    }

    /// Record the cost experienced by the internal party in an external match
    /// due to the spread between the match price and the reference price at the
    /// time of settlement
    /// 1. We don't have to remove the effects of gas sponsorship from the match
    ///    result, as it is parsed from settlement calldata, where sponsorship
    ///    is already factored out.
    /// 2. We use the match base/quote amounts to compute the trade price, as
    ///    opposed to the send/receive amounts on the bundle. This is so that we
    ///    don't factor fees into the spread. We are interested in the cost
    ///    purely due to price drift between price sampling for the quote and
    ///    settlement.
    /// 3. We sample a reference price w/in this method, meaning it should be
    ///    called as close to the actual time of settlement as possible.
    pub async fn record_external_match_spread_cost(
        &self,
        ctx: &BundleContext,
        match_result: &ApiExternalMatchResult,
    ) -> Result<(), AuthServerError> {
        let reference_price =
            self.price_reporter_client.get_price(&match_result.base_mint, self.chain).await?;

        let base_token = Token::from_addr_on_chain(&match_result.base_mint, self.chain);
        let quote_token = Token::from_addr_on_chain(&match_result.quote_mint, self.chain);

        let base_amount_decimal = base_token.convert_to_decimal(match_result.base_amount);
        let quote_amount_decimal = quote_token.convert_to_decimal(match_result.quote_amount);

        let match_price = quote_amount_decimal / base_amount_decimal;

        // The internal party takes the *opposite* side of the direction specified in
        // the `ApiExternalMatchResult`, so we must record spread cost from
        // their perspective accordingly
        let trade_side_factor = match match_result.direction.opposite() {
            OrderSide::Buy => 1.0,
            OrderSide::Sell => -1.0,
        };

        let relative_spread = trade_side_factor * (match_price - reference_price) / reference_price;
        let spread_cost = quote_amount_decimal * relative_spread;

        let side_tag_value = match match_result.direction {
            OrderSide::Buy => "buy",
            OrderSide::Sell => "sell",
        };
        let asset_tag_value = base_token.get_ticker().unwrap_or(base_token.get_addr());

        let labels = vec![
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.clone()),
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.key_description.clone()),
            (ASSET_METRIC_TAG.to_string(), asset_tag_value),
            (SIDE_TAG.to_string(), side_tag_value.to_string()),
        ];

        metrics::gauge!(EXTERNAL_MATCH_SPREAD_COST, &labels).set(spread_cost);

        Ok(())
    }

    /// Increment the token balance for a given API user
    pub async fn add_bundle_rate_limit_token(
        &self,
        key_description: &str,
    ) -> Result<(), AuthServerError> {
        self.rate_limiter.add_bundle_token(key_description).await
    }

    /// Record the gas sponsorship rate limit & metrics for a given settled
    /// match
    pub async fn record_settled_match_sponsorship(
        &self,
        ctx: &BundleContext,
        match_result: &ApiExternalMatchResult,
        receipt: &TransactionReceipt,
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
        let nominal_price = self
            .price_reporter_client
            .get_nominal_price(&refund_asset.get_addr(), self.chain)
            .await?;

        // We fetch the actual refund amount in the transaction. This is resilient
        // against:
        // 1. Bundle ID collisions resulting in us fetching incorrect gas sponsorship
        //   info
        // 2. Insufficient funds in the gas sponsor, resulting in a fallback to an
        //   unsponsored match
        let actual_refund_amount = self.get_actual_refund_amount(receipt);

        let nominal_amount: BigDecimal = actual_refund_amount.into();

        let value_bigdecimal = nominal_amount * nominal_price;

        let value = value_bigdecimal.to_f64().ok_or(AuthServerError::gas_sponsorship(
            "failed to convert gas sponsorship value to f64",
        ))?;

        self.rate_limiter.record_gas_sponsorship(&ctx.key_description, value).await?;
        self.record_gas_sponsorship_metrics(
            value,
            gas_sponsorship_info.refund_native_eth,
            actual_refund_amount,
            refund_asset,
            ctx.key_description.clone(),
            ctx.request_id.clone(),
            ctx.sdk_version.clone(),
        )
        .await?;

        Ok(())
    }

    /// Record the dollar value of sponsored gas for a settled match
    #[allow(clippy::too_many_arguments)]
    async fn record_gas_sponsorship_metrics(
        &self,
        gas_sponsorship_value: f64,
        refund_native_eth: bool,
        refund_amount: U256,
        refund_asset: Token,
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

        let refund_asset_ticker = if refund_native_eth {
            ETH_TICKER.to_string()
        } else {
            refund_asset
                .get_ticker()
                .ok_or(AuthServerError::gas_cost_sampler(REFUND_ASSET_TICKER_ERROR_MSG))?
        };

        let refund_amount_u128 =
            refund_amount.try_into().map_err(AuthServerError::gas_sponsorship)?;

        let refund_amount_whole = refund_asset.convert_to_decimal(refund_amount_u128);

        let estimate = self
            .gas_cost_sampler
            .sample_gas_prices()
            .await
            .map_err(AuthServerError::gas_sponsorship)?;

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id),
            (SDK_VERSION_METRIC_TAG.to_string(), sdk_version),
            (REFUND_ASSET_TAG.to_string(), refund_asset_ticker),
            (REFUND_AMOUNT_TAG.to_string(), refund_amount_whole.to_string()),
            (L2_BASE_FEE_TAG.to_string(), estimate.l2_base_fee.to_string()),
            (L1_COST_PER_BYTE_TAG.to_string(), estimate.l1_data_fee.to_string()),
        ];

        metrics::gauge!(GAS_SPONSORSHIP_VALUE, &labels).set(gas_sponsorship_value);

        Ok(())
    }

    /// Record the time between the canonical exchange midpoint sample time and
    /// the time of settlement
    pub fn record_settlement_delay(&self, settlement_time: u64, ctx: &BundleContext) {
        // Get the price sample time
        let price_timestamp = ctx.price_timestamp;

        // Calculate and record the time difference
        let labels = self.get_labels(ctx);
        self.record_time_diff(
            price_timestamp,
            settlement_time,
            EXTERNAL_MATCH_SETTLEMENT_DELAY,
            &labels,
        );
    }

    /// Record the time between the canonical exchange midpoint sample time and
    /// the time of assembly
    pub fn record_assembly_delay(&self, ctx: &BundleContext) {
        if let Some(assembled_timestamp) = ctx.assembled_timestamp {
            let price_timestamp = ctx.price_timestamp;
            let labels = self.get_labels(ctx);
            self.record_time_diff(
                price_timestamp,
                assembled_timestamp,
                EXTERNAL_MATCH_ASSEMBLY_DELAY,
                &labels,
            );
        }
    }

    /// Record the time between the time of assembly and the time of settlement
    pub fn record_assembly_to_settlement_delay(&self, settlement_time: u64, ctx: &BundleContext) {
        if let Some(assembled_timestamp) = ctx.assembled_timestamp {
            let labels = self.get_labels(ctx);
            self.record_time_diff(
                assembled_timestamp,
                settlement_time,
                EXTERNAL_MATCH_ASSEMBLY_TO_SETTLEMENT_DELAY,
                &labels,
            );
        }
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

    /// Returns true iff this settlement tx emitted at least one CowSwap `Trade`
    /// event—i.e. it was filled via a CowSwap batch auction.
    fn detect_cowswap_settlement(&self, receipt: &TransactionReceipt) -> bool {
        // If we can decode any `Trade` log, it came from CowSwap Settlement
        receipt.decoded_log::<GPv2Settlement::Trade>().is_some()
    }

    /// Returns the timestamp of the settlement of a bundle in milliseconds
    pub(crate) async fn get_settlement_timestamp(
        &self,
        receipt: &TransactionReceipt,
    ) -> Result<u64, AuthServerError> {
        let provider = self.darkpool_client().provider();

        let block_number = receipt
            .block_number
            .ok_or_else(|| AuthServerError::darkpool_client("receipt has no block_number"))?;

        let block = provider
            .get_block(block_number.into())
            .await
            .map_err(AuthServerError::darkpool_client)?
            .ok_or_else(|| AuthServerError::darkpool_client("block not found"))?;

        let settlement_time = block.header.timestamp * 1000; // Convert to milliseconds

        Ok(settlement_time)
    }

    /// Calculate and record the time between two timestamps
    fn record_time_diff(
        &self,
        t1: u64,
        t2: u64,
        metric_name: &'static str,
        labels: &[(String, String)],
    ) {
        let delta = t2.saturating_sub(t1);
        metrics::gauge!(metric_name, labels).set(delta as f64);
    }

    /// Get the actual refund amount sent by the gas sponsor for a given
    /// settlement transaction
    fn get_actual_refund_amount(&self, receipt: &TransactionReceipt) -> U256 {
        for log in receipt.logs() {
            if log.address() != self.gas_sponsor_address {
                continue;
            }

            if let Ok(sponsorship_log) = SponsoredExternalMatch::decode_log(&log.inner) {
                return sponsorship_log.amount;
            };
        }

        U256::ZERO
    }
}
