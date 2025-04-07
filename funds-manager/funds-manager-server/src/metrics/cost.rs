//! Defines functionality to compute and record data for swap execution

use ethers::types::{Address, TransactionReceipt, U256};
use funds_manager_api::quoters::ExecutionQuote;
use tracing::warn;

use super::MetricsRecorder;
use crate::{
    error::FundsManagerError,
    helpers::{TransferFilter, ERC20},
    metrics::labels::{
        ASSET_TAG, HASH_TAG, SWAP_EXECUTION_COST_METRIC_NAME, SWAP_NOTIONAL_VOLUME_METRIC_NAME,
        SWAP_RELATIVE_SPREAD_METRIC_NAME, TRADE_SIDE_FACTOR_TAG,
    },
};

/// Represents the cost metrics for a swap operation
#[derive(Debug)]
pub struct SwapExecutionMetrics {
    /// Net execution cost in USDC
    pub execution_cost_usdc: f64,
    /// Notional volume in USDC
    pub notional_volume_usdc: f64,
    /// Relative spread in decimal format
    pub relative_spread: f64,
}

// --------------------
// | Public Interface |
// --------------------

impl MetricsRecorder {
    /// Record the cost metrics for a swap operation
    pub async fn record_swap_cost(&self, receipt: &TransactionReceipt, quote: &ExecutionQuote) {
        match self.compute_swap_execution_metrics(receipt, quote).await {
            Ok(metrics) => {
                self.record_swap_metrics(receipt, quote, &metrics);
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
    /// Compute the cost metrics for a completed swap transaction
    async fn compute_swap_execution_metrics(
        &self,
        receipt: &TransactionReceipt,
        quote: &ExecutionQuote,
    ) -> Result<SwapExecutionMetrics, FundsManagerError> {
        let mint = quote.get_base_token().get_ethers_address();

        let binance_price = self.get_binance_price(&mint).await?;
        let transfer_amount = self.get_transfer_amount(receipt, mint, quote.from).await?;

        let execution_price = quote.get_price(Some(transfer_amount));
        let notional_volume_usdc = quote.notional_volume_usdc(transfer_amount);

        let trade_side_factor = if quote.is_buy() { 1.0 } else { -1.0 };

        let relative_spread = trade_side_factor * (execution_price - binance_price) / binance_price;
        let execution_cost_usdc = notional_volume_usdc * relative_spread;

        Ok(SwapExecutionMetrics { relative_spread, notional_volume_usdc, execution_cost_usdc })
    }

    /// Record the cost metrics for a swap operation
    fn record_swap_metrics(
        &self,
        receipt: &TransactionReceipt,
        quote: &ExecutionQuote,
        metrics: &SwapExecutionMetrics,
    ) {
        let labels = self.get_labels(quote, receipt);

        metrics::gauge!(SWAP_EXECUTION_COST_METRIC_NAME, &labels).set(metrics.execution_cost_usdc);

        metrics::gauge!(SWAP_NOTIONAL_VOLUME_METRIC_NAME, &labels)
            .set(metrics.notional_volume_usdc);

        metrics::gauge!(SWAP_RELATIVE_SPREAD_METRIC_NAME, &labels).set(metrics.relative_spread);
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
    async fn get_transfer_amount(
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
}
