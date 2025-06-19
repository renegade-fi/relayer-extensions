//! Defines functionality to compute and record data for swap execution

use alloy::rpc::types::TransactionReceipt;
use alloy_primitives::{Address, Log, TxHash, U256};
use alloy_sol_types::SolEvent;
use funds_manager_api::{quoters::AugmentedExecutionQuote, u256_try_into_u128};
use renegade_common::types::{chain::Chain, token::Token};
use renegade_darkpool_client::conversion::u256_to_amount;
use serde::Serialize;
use tracing::{info, warn};

use super::MetricsRecorder;
use crate::{
    error::FundsManagerError,
    helpers::{to_env_agnostic_name, IERC20::Transfer},
    metrics::labels::{
        ASSET_TAG, CHAIN_TAG, HASH_TAG, SWAP_EXECUTION_COST_METRIC_NAME, SWAP_GAS_COST_METRIC_NAME,
        SWAP_NOTIONAL_VOLUME_METRIC_NAME, SWAP_RELATIVE_SPREAD_METRIC_NAME, TRADE_SIDE_FACTOR_TAG,
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
    /// The gas cost of execution in USD
    pub gas_cost_usd: f64,

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
    pub async fn record_swap_cost(
        &self,
        receipt: &TransactionReceipt,
        quote: &AugmentedExecutionQuote,
        swap_gas_cost: U256,
    ) -> Result<SwapExecutionData, FundsManagerError> {
        let cost_data = match self.build_swap_cost_data(receipt, quote, swap_gas_cost).await {
            Ok(cost_data) => cost_data,
            Err(e) => {
                warn!("Failed to build swap cost data: {e}");
                return Err(e);
            },
        };

        // Record metrics from the cost data
        self.record_metrics_from_cost_data(receipt, quote, &cost_data);
        self.log_swap_cost_data(&cost_data, receipt.transaction_hash);

        Ok(cost_data)
    }
}

// -------------------
// | Private Methods |
// -------------------

impl MetricsRecorder {
    /// Build the unified swap cost data from available information
    async fn build_swap_cost_data(
        &self,
        receipt: &TransactionReceipt,
        quote: &AugmentedExecutionQuote,
        swap_gas_cost: U256,
    ) -> Result<SwapExecutionData, FundsManagerError> {
        let base_mint = quote.get_base_token().get_alloy_address();
        let binance_price = self.get_price(&base_mint, quote.chain).await?;

        let buy_mint = quote.get_buy_token().get_alloy_address();
        let buy_amount_actual = self.get_buy_amount_actual(receipt, buy_mint, quote.quote.from)?;

        let execution_price =
            quote.get_price(Some(buy_amount_actual)).map_err(FundsManagerError::parse)?;
        let gas_cost_usd = self.get_wei_cost_usdc(swap_gas_cost).await?;
        let notional_volume_usdc =
            quote.notional_volume_usdc(buy_amount_actual).map_err(FundsManagerError::parse)?;

        let trade_side_factor = if quote.is_buy() { 1.0 } else { -1.0 };
        let relative_spread = trade_side_factor * (execution_price - binance_price) / binance_price;
        let execution_cost_usdc = notional_volume_usdc * relative_spread;

        // Calculate slippage metrics
        let decimal_corrected_buy_amount_estimated =
            quote.get_decimal_corrected_buy_amount().map_err(FundsManagerError::parse)?;
        let decimal_corrected_buy_amount_min =
            quote.get_decimal_corrected_buy_amount_min().map_err(FundsManagerError::parse)?;

        let buy_amount_actual =
            u256_try_into_u128(buy_amount_actual).map_err(FundsManagerError::parse)?;
        let decimal_corrected_buy_amount_actual =
            quote.get_buy_token().convert_to_decimal(buy_amount_actual);

        let sell_amount =
            quote.get_decimal_corrected_sell_amount().map_err(FundsManagerError::parse)?;

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
            sell_amount: sell_amount.to_string(),
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
            gas_cost_usd,

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
        quote: &AugmentedExecutionQuote,
        cost_data: &SwapExecutionData,
    ) {
        let labels = self.get_labels(quote, receipt);

        metrics::gauge!(SWAP_EXECUTION_COST_METRIC_NAME, &labels)
            .set(cost_data.execution_cost_usdc);
        metrics::gauge!(SWAP_GAS_COST_METRIC_NAME, &labels).set(cost_data.gas_cost_usd);
        metrics::gauge!(SWAP_NOTIONAL_VOLUME_METRIC_NAME, &labels)
            .set(cost_data.notional_volume_usdc);
        metrics::gauge!(SWAP_RELATIVE_SPREAD_METRIC_NAME, &labels).set(cost_data.relative_spread);
    }

    /// Derive the labels given a quote and a transaction receipt
    fn get_labels(
        &self,
        quote: &AugmentedExecutionQuote,
        receipt: &TransactionReceipt,
    ) -> Vec<(String, String)> {
        let mint = format!("{:#x}", quote.get_base_token().get_alloy_address());
        let asset = quote.get_base_token().get_ticker().unwrap_or(mint);
        let side_label = if quote.is_buy() { "buy" } else { "sell" };
        let chain = to_env_agnostic_name(self.chain);

        vec![
            (ASSET_TAG.to_string(), asset),
            (TRADE_SIDE_FACTOR_TAG.to_string(), side_label.to_string()),
            (HASH_TAG.to_string(), format!("{:#x}", receipt.transaction_hash)),
            (CHAIN_TAG.to_string(), chain),
        ]
    }

    /// Extract the transfer amount from a transaction receipt
    fn get_buy_amount_actual(
        &self,
        receipt: &TransactionReceipt,
        mint: Address,
        recipient: Address,
    ) -> Result<U256, FundsManagerError> {
        let logs: Vec<Log<Transfer>> = receipt
            .logs()
            .iter()
            .filter_map(|log| {
                if log.address() != mint {
                    None
                } else {
                    Transfer::decode_log(&log.inner).ok()
                }
            })
            .collect();

        logs.iter()
            .find_map(|transfer| if transfer.to == recipient { Some(transfer.value) } else { None })
            .ok_or(FundsManagerError::on_chain("no matching transfer event found"))
    }

    /// Get the price for a token
    async fn get_price(&self, mint: &Address, chain: Chain) -> Result<f64, FundsManagerError> {
        let price = self.price_reporter.get_price(&format!("{:#x}", mint), chain).await?;
        Ok(price)
    }

    /// Get the cost of the given amount of WEI in USD
    async fn get_wei_cost_usdc(&self, wei: U256) -> Result<f64, FundsManagerError> {
        // We use the weth price here as a stand in for the eth price seeing as weth
        // does not trade at a discount to eth. As well, we fetch the price using the
        // arbitrum one chain for simplicity -- the price will be the same across chains
        let weth = Token::from_ticker_on_chain("WETH", Chain::ArbitrumOne);
        let price = self.get_price(&weth.get_alloy_address(), Chain::ArbitrumOne).await?;

        // Convert the wei to weth then to usdc
        let wei_u128 = u256_to_amount(wei).map_err(FundsManagerError::parse)?;
        let weth_input = weth.convert_to_decimal(wei_u128);
        let cost = weth_input * price;
        Ok(cost)
    }

    /// Log swap cost data in a Datadog-compatible format
    fn log_swap_cost_data(&self, cost_data: &SwapExecutionData, tx_hash: TxHash) {
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
            gas_cost_usd = %cost_data.gas_cost_usd,
            notional_volume_usdc = %cost_data.notional_volume_usdc,
            relative_spread = %cost_data.relative_spread,
            execution_cost_usdc = %cost_data.execution_cost_usdc,
            slippage_budget = %cost_data.slippage_budget,
            received_delta = %cost_data.received_delta,
            slippage_consumption_percent = %cost_data.slippage_consumption_percent,
            chain = %to_env_agnostic_name(self.chain),
            "Swap recorded for tx {tx_hash:#x}");
    }
}
