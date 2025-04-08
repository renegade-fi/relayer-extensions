//! Defines functionality to compute and record data for swap execution

use ethers::types::{Address, TransactionReceipt, U256};
use funds_manager_api::quoters::ExecutionQuote;
use serde::Serialize;
use tracing::{info, warn};

use super::MetricsRecorder;
use crate::{
    error::FundsManagerError,
    helpers::{TransactionHash, TransferFilter, ERC20},
    metrics::labels::{
        ASSET_TAG, HASH_TAG, SWAP_EXECUTION_COST_METRIC_NAME, SWAP_NOTIONAL_VOLUME_METRIC_NAME,
        SWAP_RELATIVE_SPREAD_METRIC_NAME, TRADE_SIDE_FACTOR_TAG,
    },
};

/// Unified data structure for swap cost information
#[derive(Debug, Serialize)]
pub struct SwapExecutionData {
    // Token information
    /// The token address being bought
    pub buy_token_address: String,
    /// The token address being sold
    pub sell_token_address: String,
    /// The amount of tokens to sell
    pub sell_amount: String,
    /// The estimated amount of tokens to be received
    pub buy_amount_estimated: String,
    /// The minimum amount of tokens to be received
    pub buy_amount_min: String,
    /// The address initiating the swap
    pub from_address: String,
    /// The address receiving the swap
    pub to_address: String,
    /// Whether the swap is a buy or sell
    pub is_buy: bool,

    // Market reference data
    /// The Binance price for the token
    pub binance_price: f64,
    /// The transaction hash
    pub transaction_hash: String,

    // Execution details
    /// The actual transfer amount from the transaction
    pub buy_amount_actual: String,
    /// The execution price in USDC per token
    pub execution_price: f64,
    /// The notional volume in USDC
    pub notional_volume_usdc: f64,
    /// The relative spread in decimal format
    pub relative_spread: f64,
    /// The execution cost in USDC
    pub execution_cost_usdc: f64,

    // Slippage information
    /// The slippage budget (difference between estimated and minimum)
    pub slippage_budget: f64,
    /// The difference between estimated and actual amount received
    pub received_delta: f64,
    /// The percentage of slippage budget consumed
    pub slippage_consumption_percent: f64,
}

// --------------------
// | Public Interface |
// --------------------

impl MetricsRecorder {
    /// Record the cost metrics for a swap operation
    pub async fn record_swap_cost(&self, receipt: &TransactionReceipt, quote: &ExecutionQuote) {
        match self.build_swap_cost_data(receipt, quote).await {
            Ok(cost_data) => {
                // Record metrics from the cost data
                self.record_metrics_from_cost_data(receipt, quote, &cost_data);

                // Log the cost data
                self.log_swap_cost_data(&cost_data, receipt.transaction_hash);
            },
            Err(e) => {
                warn!("Failed to record swap cost for tx {}: {}", receipt.transaction_hash, e);
            },
        }
    }
}

// --------------------
// | Private Methods |
// --------------------

impl MetricsRecorder {
    /// Build the unified swap cost data from available information
    async fn build_swap_cost_data(
        &self,
        receipt: &TransactionReceipt,
        quote: &ExecutionQuote,
    ) -> Result<SwapExecutionData, FundsManagerError> {
        let mint = quote.get_base_token().get_ethers_address();

        let binance_price = self.get_binance_price(&mint).await?;
        let buy_amount_actual = self.get_buy_amount_actual(receipt, mint, quote.from).await?;

        let execution_price = quote.get_price(Some(buy_amount_actual));
        let notional_volume_usdc = quote.notional_volume_usdc(buy_amount_actual);

        let trade_side_factor = if quote.is_buy() { 1.0 } else { -1.0 };
        let relative_spread = trade_side_factor * (execution_price - binance_price) / binance_price;
        let execution_cost_usdc = notional_volume_usdc * relative_spread;

        // Calculate slippage metrics
        let decimal_corrected_buy_amount_estimated = quote.get_decimal_corrected_buy_amount();
        let decimal_corrected_buy_amount_min = quote.get_decimal_corrected_buy_amount_min();
        let decimal_corrected_buy_amount_actual =
            quote.get_buy_token().convert_to_decimal(buy_amount_actual.as_u128());

        let slippage_budget =
            decimal_corrected_buy_amount_estimated - decimal_corrected_buy_amount_min;
        let received_delta =
            decimal_corrected_buy_amount_estimated - decimal_corrected_buy_amount_actual;
        let slippage_consumption_percent =
            if slippage_budget > 0.0 { (received_delta / slippage_budget) * 100.0 } else { 0.0 };

        // Create and return the unified cost data
        Ok(SwapExecutionData {
            // Token information
            buy_token_address: quote.get_buy_token_address(),
            sell_token_address: quote.get_sell_token_address(),
            sell_amount: quote.get_decimal_corrected_sell_amount().to_string(),
            buy_amount_estimated: decimal_corrected_buy_amount_estimated.to_string(),
            buy_amount_min: decimal_corrected_buy_amount_min.to_string(),
            from_address: quote.get_from_address(),
            to_address: quote.get_to_address(),
            is_buy: quote.is_buy(),

            // Market reference data
            binance_price,
            transaction_hash: format!("{:#x}", receipt.transaction_hash),

            // Execution details
            buy_amount_actual: decimal_corrected_buy_amount_actual.to_string(),
            execution_price,
            notional_volume_usdc,
            relative_spread,
            execution_cost_usdc,

            // Slippage information
            slippage_budget,
            received_delta,
            slippage_consumption_percent,
        })
    }

    /// Record metrics from the unified cost data
    fn record_metrics_from_cost_data(
        &self,
        receipt: &TransactionReceipt,
        quote: &ExecutionQuote,
        cost_data: &SwapExecutionData,
    ) {
        let labels = self.get_labels(quote, receipt);

        metrics::gauge!(SWAP_EXECUTION_COST_METRIC_NAME, &labels)
            .set(cost_data.execution_cost_usdc);
        metrics::gauge!(SWAP_NOTIONAL_VOLUME_METRIC_NAME, &labels)
            .set(cost_data.notional_volume_usdc);
        metrics::gauge!(SWAP_RELATIVE_SPREAD_METRIC_NAME, &labels).set(cost_data.relative_spread);
    }

    /// Derive the labels given a quote and a transaction receipt
    fn get_labels(
        &self,
        quote: &ExecutionQuote,
        receipt: &TransactionReceipt,
    ) -> Vec<(String, String)> {
        let mint = format!("{:#x}", quote.get_base_token().get_ethers_address());
        let asset = quote.get_base_token().get_ticker().unwrap_or(mint);
        let side_label = if quote.is_buy() { "buy" } else { "sell" };

        vec![
            (ASSET_TAG.to_string(), asset),
            (TRADE_SIDE_FACTOR_TAG.to_string(), side_label.to_string()),
            (HASH_TAG.to_string(), format!("{:#x}", receipt.transaction_hash)),
        ]
    }

    /// Extract the transfer amount from a transaction receipt
    async fn get_buy_amount_actual(
        &self,
        receipt: &TransactionReceipt,
        mint: Address,
        recipient: Address,
    ) -> Result<U256, FundsManagerError> {
        let block_number = receipt.block_number.unwrap_or_default().as_u64();

        let contract = ERC20::new(mint, self.provider.clone());
        let filter = contract
            .event::<TransferFilter>()
            .from_block(block_number)
            .to_block(block_number)
            .topic2(recipient);

        let events = filter
            .query_with_meta()
            .await
            .map_err(|_| FundsManagerError::arbitrum("failed to create transfer stream"))?;

        // Find the transfer event that matches our transaction hash
        let transfer_event = events
            .iter()
            .find(|(_, meta)| meta.transaction_hash == receipt.transaction_hash)
            .ok_or_else(|| FundsManagerError::custom("No matching transfer event found"))?;

        Ok(transfer_event.0.value)
    }

    /// Get the Binance price for a token
    async fn get_binance_price(&self, mint: &Address) -> Result<f64, FundsManagerError> {
        let price = self.relayer_client.get_binance_price(&format!("{:#x}", mint)).await?;
        price.ok_or_else(|| FundsManagerError::custom("No Binance price available for token"))
    }

    /// Log swap cost data in a Datadog-compatible format
    fn log_swap_cost_data(&self, cost_data: &SwapExecutionData, tx_hash: TransactionHash) {
        info!(
            buy_token_address = %cost_data.buy_token_address,
            sell_token_address = %cost_data.sell_token_address,
            sell_amount = %cost_data.sell_amount,
            buy_amount_estimated = %cost_data.buy_amount_estimated,
            buy_amount_min = %cost_data.buy_amount_min,
            from_address = %cost_data.from_address,
            to_address = %cost_data.to_address,
            is_buy = %cost_data.is_buy,
            binance_price = %cost_data.binance_price,
            transaction_hash = %cost_data.transaction_hash,
            buy_amount_actual = %cost_data.buy_amount_actual,
            execution_price = %cost_data.execution_price,
            notional_volume_usdc = %cost_data.notional_volume_usdc,
            relative_spread = %cost_data.relative_spread,
            execution_cost_usdc = %cost_data.execution_cost_usdc,
            slippage_budget = %cost_data.slippage_budget,
            received_delta = %cost_data.received_delta,
            slippage_consumption_percent = %cost_data.slippage_consumption_percent,
            "Swap recorded for tx {tx_hash:#x}");
    }
}
