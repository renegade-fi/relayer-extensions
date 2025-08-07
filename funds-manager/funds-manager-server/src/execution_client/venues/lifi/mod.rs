//! Lifi-specific logic for getting quotes and executing swaps.
//!
//! Includes definitions for the Lifi API types, as defined in
//! <https://apidocs.li.fi/reference/get_v1-quote>

use alloy::{
    eips::BlockId,
    hex,
    network::TransactionBuilder,
    providers::{DynProvider, Provider},
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{Address, Bytes, Log, U256};
use alloy_sol_types::SolEvent;
use async_trait::async_trait;
use funds_manager_api::{quoters::QuoteParams, u256_try_into_u64};
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::Deserialize;
use tracing::{error, info, instrument, warn};

use crate::{
    execution_client::{
        error::ExecutionClientError,
        swap::DEFAULT_SLIPPAGE_TOLERANCE,
        venues::{
            lifi::api_types::{LifiQuote, LifiQuoteParams},
            quote::{ExecutableQuote, ExecutionQuote, QuoteExecutionData},
            ExecutionResult, ExecutionVenue, SupportedExecutionVenue,
        },
    },
    helpers::{
        approve_erc20_allowance, build_provider, get_gas_cost, handle_http_response,
        send_tx_with_retry, to_chain_id, IERC20::Transfer, ONE_CONFIRMATION,
    },
};

pub mod api_types;

// -------------
// | Constants |
// -------------

/// The base URL for the Lifi API
const LIFI_BASE_URL: &str = "https://li.quest/v1";

/// The endpoint for getting a quote
const LIFI_QUOTE_ENDPOINT: &str = "quote";

/// The address of the LiFi diamond (same address on Arbitrum One and Base
/// Mainnet), constantized here to simplify approvals
const LIFI_DIAMOND_ADDRESS: Address =
    Address::new(hex!("0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae"));

/// The Lifi api key header
const LIFI_API_KEY_HEADER: &str = "x-lifi-api-key";

/// The default max price impact for a Lifi quote.
///
/// Note that we validate price impact on our own before executing a quote,
/// as Lifi sometimes mis-prices tokens (e.g. $EDGE)
const DEFAULT_MAX_PRICE_IMPACT: f64 = 0.3; // 30%

/// The default swap step timing strategy for a Lifi quote.
///
/// See <https://docs.li.fi/guides/integration-tips/latency#timing-strategy-format> for
/// more details.
const DEFAULT_TIMING_STRATEGY: &str = "minWaitTime-600-4-300";

/// The default order preference for a Lifi quote.
///
/// See https://docs.li.fi/api-reference/get-a-quote-for-a-token-transfer#parameter-order for
/// more details.
const DEFAULT_ORDER_PREFERENCE: &str = "CHEAPEST";

/// The default denied exchanges for a Lifi quote.
///
/// See <https://docs.li.fi/api-reference/get-a-quote-for-a-token-transfer#parameter-deny-exchanges>
/// for more details.
const DEFAULT_DENIED_EXCHANGES: [&str; 1] = ["sushiswap"];

// ---------
// | Types |
// ---------

/// Lifi-specific quote execution data
#[derive(Debug, Clone)]
pub struct LifiQuoteExecutionData {
    /// The swap contract address
    pub to: Address,
    /// The submitting address
    pub from: Address,
    /// The value of the tx; should be zero
    pub value: U256,
    /// The calldata for the swap
    pub data: Bytes,
    /// The gas limit for the swap
    pub gas_limit: U256,
    /// The tool (venue) providing the route
    pub tool: String,
}

impl ExecutableQuote {
    /// Convert a LiFi quote into an executable quote
    pub fn from_lifi_quote(
        lifi_quote: LifiQuote,
        chain: Chain,
    ) -> Result<Self, ExecutionClientError> {
        let sell_token = lifi_quote.get_sell_token(chain);
        let buy_token = lifi_quote.get_buy_token(chain);
        let sell_amount = lifi_quote.get_sell_amount()?;
        let buy_amount = lifi_quote.get_buy_amount()?;

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Lifi,
            chain,
        };

        let to = lifi_quote.get_to_address()?;
        let from = lifi_quote.get_from_address()?;
        let value = lifi_quote.get_value()?;
        let data = lifi_quote.get_data()?;
        let gas_limit = lifi_quote.get_gas_limit()?;
        let tool = lifi_quote.get_tool();

        let execution_data = LifiQuoteExecutionData { to, from, value, data, gas_limit, tool };

        Ok(ExecutableQuote { quote, execution_data: QuoteExecutionData::Lifi(execution_data) })
    }
}

// ----------
// | Client |
// ----------

/// A client for interacting with the Lifi API
#[derive(Clone)]
pub struct LifiClient {
    /// The API key to use for requests
    api_key: Option<String>,
    /// The underlying HTTP client
    http_client: Client,
    /// The RPC provider
    rpc_provider: DynProvider,
    /// The address of the hot wallet used for executing quotes
    hot_wallet_address: Address,
    /// The chain on which the client is operating
    chain: Chain,
}

impl LifiClient {
    /// Create a new client
    pub fn new(
        api_key: Option<String>,
        rpc_url: &str,
        hot_wallet: PrivateKeySigner,
        chain: Chain,
    ) -> Self {
        let hot_wallet_address = hot_wallet.address();
        let rpc_provider = build_provider(rpc_url, Some(hot_wallet));

        Self { api_key, http_client: Client::new(), rpc_provider, hot_wallet_address, chain }
    }

    /// Send a get request to the execution venue
    async fn send_get_request<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, ExecutionClientError> {
        let url = format!("{LIFI_BASE_URL}/{path}");

        // Add an API key if present
        let mut request = self.http_client.get(url);
        if let Some(api_key) = &self.api_key {
            request = request.header(LIFI_API_KEY_HEADER, api_key.as_str());
        }

        let response = request.send().await?;
        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Construct Lifi quote parameters from a venue-agnostic quote params
    /// object, with reasonable defaults.
    fn construct_quote_params(&self, params: QuoteParams) -> LifiQuoteParams {
        let deny_exchanges = DEFAULT_DENIED_EXCHANGES.into_iter().map(String::from).collect();

        LifiQuoteParams {
            from_token: params.from_token,
            to_token: params.to_token,
            from_amount: params.from_amount,
            from_address: self.hot_wallet_address.to_string(),
            from_chain: to_chain_id(self.chain) as usize,
            to_chain: to_chain_id(self.chain) as usize,
            slippage: params.slippage_tolerance.or(Some(DEFAULT_SLIPPAGE_TOLERANCE)),
            max_price_impact: Some(DEFAULT_MAX_PRICE_IMPACT),
            swap_step_timing_strategies: Some(vec![DEFAULT_TIMING_STRATEGY.to_string()]),
            order: Some(DEFAULT_ORDER_PREFERENCE.to_string()),
            skip_simulation: Some(true),
            deny_exchanges: Some(deny_exchanges),
            ..Default::default()
        }
    }

    /// Approve an erc20 allowance for the Lifi diamond
    #[instrument(skip(self))]
    async fn approve_erc20_allowance(
        &self,
        token_address: Address,
        amount: U256,
    ) -> Result<(), ExecutionClientError> {
        approve_erc20_allowance(
            token_address,
            LIFI_DIAMOND_ADDRESS,
            self.hot_wallet_address,
            amount,
            self.rpc_provider.clone(),
        )
        .await
        .map_err(ExecutionClientError::onchain)
    }

    /// Construct a swap transaction from Lifi execution data
    async fn build_swap_tx(
        &self,
        execution_data: &LifiQuoteExecutionData,
    ) -> Result<TransactionRequest, ExecutionClientError> {
        let latest_block = self
            .rpc_provider
            .get_block(BlockId::latest())
            .await
            .map_err(ExecutionClientError::onchain)?
            .ok_or(ExecutionClientError::onchain("No latest block found"))?;

        let latest_basefee = latest_block
            .header
            .base_fee_per_gas
            .ok_or(ExecutionClientError::onchain("No basefee found"))?
            as u128;

        let gas_limit =
            u256_try_into_u64(execution_data.gas_limit).map_err(ExecutionClientError::onchain)?;

        let tx = TransactionRequest::default()
            .with_to(execution_data.to)
            .with_from(execution_data.from)
            .with_value(execution_data.value)
            .with_input(execution_data.data.clone())
            .with_max_fee_per_gas(latest_basefee * 2)
            .with_max_priority_fee_per_gas(latest_basefee * 2)
            .with_gas_limit(gas_limit);

        Ok(tx)
    }

    /// Send an onchain transaction with the configured RPC provider
    /// (expected to be configured with a signer)
    async fn send_tx(
        &self,
        tx: TransactionRequest,
    ) -> Result<TransactionReceipt, ExecutionClientError> {
        send_tx_with_retry(tx, &self.rpc_provider, ONE_CONFIRMATION)
            .await
            .map_err(ExecutionClientError::onchain)
    }

    /// Extract the transfer amount from a transaction receipt
    fn get_buy_amount_actual(
        &self,
        receipt: &TransactionReceipt,
        buy_token_address: Address,
        recipient: Address,
    ) -> Result<U256, ExecutionClientError> {
        let logs: Vec<Log<Transfer>> = receipt
            .logs()
            .iter()
            .filter_map(|log| {
                if log.address() != buy_token_address {
                    None
                } else {
                    Transfer::decode_log(&log.inner).ok()
                }
            })
            .collect();

        logs.iter()
            .find_map(|transfer| if transfer.to == recipient { Some(transfer.value) } else { None })
            .ok_or(ExecutionClientError::onchain("no matching transfer event found"))
    }
}

#[async_trait]
impl ExecutionVenue for LifiClient {
    /// Get the name of the venue
    fn venue_specifier(&self) -> SupportedExecutionVenue {
        SupportedExecutionVenue::Lifi
    }

    /// Get a quote from the Lifi API
    #[instrument(skip_all)]
    async fn get_quote(
        &self,
        params: QuoteParams,
    ) -> Result<ExecutableQuote, ExecutionClientError> {
        let lifi_params = self.construct_quote_params(params);
        let qs_config = serde_qs::Config::new().array_format(serde_qs::ArrayFormat::Unindexed);
        let query_string = qs_config.serialize_string(&lifi_params).unwrap();
        let path = format!("{LIFI_QUOTE_ENDPOINT}?{query_string}");

        // Log the request path if the quote fails
        let resp: LifiQuote = match self.send_get_request(&path).await {
            Ok(resp) => resp,
            Err(e) => {
                error!("LiFi error with path: {path}: {e}");
                return Err(e);
            },
        };

        ExecutableQuote::from_lifi_quote(resp, self.chain)
    }

    /// Execute a quote from the Lifi API
    #[instrument(skip_all)]
    async fn execute_quote(
        &self,
        executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError> {
        let ExecutableQuote { quote, execution_data } = executable_quote;
        let lifi_execution_data = execution_data.lifi()?;

        self.approve_erc20_allowance(quote.sell_token.get_alloy_address(), quote.sell_amount)
            .await?;

        let tx = self.build_swap_tx(&lifi_execution_data).await?;

        info!("Executing Lifi quote from {}", lifi_execution_data.tool);

        let receipt = self.send_tx(tx).await?;
        let gas_cost = get_gas_cost(&receipt);
        let tx_hash = receipt.transaction_hash;

        if receipt.status() {
            let recipient = lifi_execution_data.from;
            let buy_token_address = quote.buy_token.get_alloy_address();
            let buy_amount_actual =
                self.get_buy_amount_actual(&receipt, buy_token_address, recipient)?;

            Ok(ExecutionResult { buy_amount_actual, gas_cost, tx_hash: Some(tx_hash) })
        } else {
            warn!("tx ({:#x}) failed", tx_hash);
            // For an unsuccessful swap, we exclude the TX hash and report
            // an actual buy amount of zero, but we still include the gas cost
            Ok(ExecutionResult { buy_amount_actual: U256::ZERO, gas_cost, tx_hash: None })
        }
    }
}
