//! Defines functionality to compute and record data for swap execution

use alloy::providers::Provider;
use alloy_primitives::{Address, TxHash, U256};
use alloy_sol_types::SolEvent;
use funds_manager_api::u256_try_into_u128;
use renegade_types_core::{Chain, Token, USDC_TICKER};
use serde::Serialize;
use tracing::{info, warn};

use super::MetricsRecorder;
use crate::{
    error::FundsManagerError,
    execution_client::{swap::DecayingSwapOutcome, venues::quote::ExecutionQuote},
    helpers::{get_darkpool_address, to_env_agnostic_name, IERC20::Transfer},
    metrics::labels::{
        ASSET_TAG, CHAIN_TAG, SELF_TRADE_VOLUME_USDC_METRIC_NAME, SWAP_EXECUTION_COST_METRIC_NAME,
        SWAP_GAS_COST_METRIC_NAME, SWAP_NOTIONAL_VOLUME_METRIC_NAME,
        SWAP_RELATIVE_SPREAD_METRIC_NAME, TRADE_SIDE_FACTOR_TAG, VENUE_TAG,
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
    /// The USDC volume transferred through the darkpool in the swap
    pub self_trade_volume_usdc: f64,
    /// The venue that executed the swap
    pub venue: String,

    // Slippage information
    /// The difference between estimated and actual amount received
    pub received_delta: f64,
}

// --------------------
// | Public Interface |
// --------------------

impl MetricsRecorder {
    /// Record the cost metrics for a swap operation
    pub async fn record_swap_cost(
        &self,
        swap_outcome: &DecayingSwapOutcome,
    ) -> Result<SwapExecutionData, FundsManagerError> {
        let cost_data = match self.build_swap_cost_data(swap_outcome).await {
            Ok(cost_data) => cost_data,
            Err(e) => {
                warn!("Failed to build swap cost data: {e}");
                return Err(e);
            },
        };

        // Record metrics from the cost data
        self.record_metrics_from_cost_data(&swap_outcome.quote, &cost_data);
        self.log_swap_cost_data(&cost_data, swap_outcome.tx_hash);

        Ok(cost_data)
    }
}

// -------------------
// | Private Methods |
// -------------------

impl MetricsRecorder {
    /// Get the darkpool address for the recorder's chain
    fn get_darkpool_address(&self) -> Address {
        get_darkpool_address(self.chain)
    }

    /// Build the unified swap cost data from available information
    async fn build_swap_cost_data(
        &self,
        swap_outcome: &DecayingSwapOutcome,
    ) -> Result<SwapExecutionData, FundsManagerError> {
        let DecayingSwapOutcome {
            quote,
            buy_amount_actual,
            tx_hash,
            cumulative_gas_cost: swap_gas_cost,
        } = swap_outcome;

        let base_mint = quote.base_token().get_alloy_address();
        let binance_price = self.get_price(&base_mint, quote.chain).await?;

        let execution_price = quote.get_price(Some(*buy_amount_actual));
        let gas_cost_usd = self.get_wei_cost_usdc(*swap_gas_cost).await?;
        let notional_volume_usdc = quote.notional_volume_usdc(*buy_amount_actual);

        let trade_side_factor = if quote.is_sell() { -1.0 } else { 1.0 };
        let relative_spread = trade_side_factor * (execution_price - binance_price) / binance_price;
        let execution_cost_usdc = notional_volume_usdc * relative_spread;

        // Calculate slippage metrics
        let decimal_corrected_buy_amount_estimated = quote.buy_amount_decimal();

        let buy_amount_actual =
            u256_try_into_u128(*buy_amount_actual).map_err(FundsManagerError::parse)?;

        let decimal_corrected_buy_amount_actual =
            quote.buy_token.convert_to_decimal(buy_amount_actual);

        let sell_amount = quote.sell_amount_decimal();

        let received_delta =
            decimal_corrected_buy_amount_estimated - decimal_corrected_buy_amount_actual;

        let self_trade_volume_usdc = self.get_self_trade_volume(*tx_hash).await?;

        // Create and return the unified cost data
        Ok(SwapExecutionData {
            // Token information
            buy_token_address: quote.buy_token.addr.clone(),
            sell_token_address: quote.sell_token.addr.clone(),
            sell_amount: sell_amount.to_string(),
            buy_amount_estimated: decimal_corrected_buy_amount_estimated.to_string(),
            is_buy: !quote.is_sell(),

            // Market reference data
            binance_price,
            transaction_hash: format!("{:#x}", tx_hash),

            // Execution details
            buy_amount_actual: decimal_corrected_buy_amount_actual.to_string(),
            execution_price,
            notional_volume_usdc,
            relative_spread,
            execution_cost_usdc,
            gas_cost_usd,
            self_trade_volume_usdc,
            venue: quote.venue.to_string(),

            // Slippage information
            received_delta,
        })
    }

    /// Record metrics from the unified cost data
    fn record_metrics_from_cost_data(&self, quote: &ExecutionQuote, cost_data: &SwapExecutionData) {
        let labels = self.get_labels(quote);

        metrics::gauge!(SWAP_EXECUTION_COST_METRIC_NAME, &labels)
            .set(cost_data.execution_cost_usdc);
        metrics::gauge!(SWAP_GAS_COST_METRIC_NAME, &labels).set(cost_data.gas_cost_usd);
        metrics::gauge!(SWAP_NOTIONAL_VOLUME_METRIC_NAME, &labels)
            .set(cost_data.notional_volume_usdc);
        metrics::gauge!(SWAP_RELATIVE_SPREAD_METRIC_NAME, &labels).set(cost_data.relative_spread);
        metrics::gauge!(SELF_TRADE_VOLUME_USDC_METRIC_NAME, &labels)
            .set(cost_data.self_trade_volume_usdc);
    }

    /// Derive the labels given a quote and a transaction receipt
    fn get_labels(&self, quote: &ExecutionQuote) -> Vec<(String, String)> {
        let base_token = quote.base_token();
        let mint = format!("{:#x}", base_token.get_alloy_address());
        let asset = base_token.get_ticker().unwrap_or(mint);
        let side_label = if quote.is_sell() { "sell" } else { "buy" };
        let chain = to_env_agnostic_name(self.chain);

        vec![
            (ASSET_TAG.to_string(), asset),
            (TRADE_SIDE_FACTOR_TAG.to_string(), side_label.to_string()),
            (CHAIN_TAG.to_string(), chain),
            (VENUE_TAG.to_string(), quote.venue.to_string()),
        ]
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
        let wei_u128 = u256_try_into_u128(wei).map_err(FundsManagerError::parse)?;
        let weth_input = weth.convert_to_decimal(wei_u128);
        let cost = weth_input * price;
        Ok(cost)
    }

    /// Get the USDC volume transferred through the darkpool in the given
    /// transaction.
    ///
    /// This function assumes the transaction hash is for a swap executed by the
    /// funds manager, in which case this represents the notional volume
    /// that we executed through our own protocol.
    async fn get_self_trade_volume(&self, tx_hash: TxHash) -> Result<f64, FundsManagerError> {
        let receipt = self
            .provider
            .get_transaction_receipt(tx_hash)
            .await
            .map_err(FundsManagerError::on_chain)?;

        if receipt.is_none() {
            return Err(FundsManagerError::on_chain(format!(
                "No receipt found for tx {tx_hash:#x}"
            )));
        }
        let receipt = receipt.unwrap();

        let usdc_token = Token::from_ticker_on_chain(USDC_TICKER, self.chain);
        let usdc_address = usdc_token.get_alloy_address();

        let darkpool_address = self.get_darkpool_address();

        receipt
            .logs()
            .iter()
            .map(|log| {
                if log.address() != usdc_address {
                    return Ok(0.0);
                }

                let transfer = match Transfer::decode_log(&log.inner) {
                    Ok(transfer) => transfer,
                    // Failure to decode implies the event is not a transfer
                    Err(_) => return Ok(0.0),
                };

                if transfer.to == darkpool_address || transfer.from == darkpool_address {
                    let value =
                        u256_try_into_u128(transfer.value).map_err(FundsManagerError::parse)?;
                    Ok(usdc_token.convert_to_decimal(value))
                } else {
                    Ok(0.0)
                }
            })
            .collect::<Result<Vec<f64>, FundsManagerError>>()
            .map(|darkpool_usdc_transfer_amounts| darkpool_usdc_transfer_amounts.iter().sum())
    }

    /// Log swap cost data in a Datadog-compatible format
    fn log_swap_cost_data(&self, cost_data: &SwapExecutionData, tx_hash: TxHash) {
        info!(
            buy_token_address = %cost_data.buy_token_address,
            sell_token_address = %cost_data.sell_token_address,
            sell_amount = %cost_data.sell_amount,
            buy_amount_estimated = %cost_data.buy_amount_estimated,
            is_buy = %cost_data.is_buy,
            binance_price = %cost_data.binance_price,
            transaction_hash = %cost_data.transaction_hash,
            buy_amount_actual = %cost_data.buy_amount_actual,
            execution_price = %cost_data.execution_price,
            gas_cost_usd = %cost_data.gas_cost_usd,
            notional_volume_usdc = %cost_data.notional_volume_usdc,
            relative_spread = %cost_data.relative_spread,
            execution_cost_usdc = %cost_data.execution_cost_usdc,
            self_trade_volume_usdc = %cost_data.self_trade_volume_usdc,
            received_delta = %cost_data.received_delta,
            chain = %to_env_agnostic_name(self.chain),
            venue = %cost_data.venue,
            "Swap recorded for tx {tx_hash:#x}");
    }
}
