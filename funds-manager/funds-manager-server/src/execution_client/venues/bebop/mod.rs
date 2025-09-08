//! Bebop-specific logic for getting quotes and executing swaps

use std::str::FromStr;

use alloy::{
    eips::BlockId,
    network::TransactionBuilder,
    providers::{DynProvider, Provider},
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{Address, Bytes, U256};
use async_trait::async_trait;
use funds_manager_api::quoters::{QuoteParams, SupportedExecutionVenue};
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, instrument, warn};

use crate::{
    execution_client::{
        error::ExecutionClientError,
        swap::DEFAULT_SLIPPAGE_TOLERANCE,
        venues::{
            bebop::api_types::{ApprovalType, BebopQuoteParams, BebopQuoteResponse},
            quote::{CrossVenueQuoteSource, ExecutableQuote, ExecutionQuote, QuoteExecutionData},
            ExecutionResult, ExecutionVenue,
        },
    },
    helpers::{
        approve_erc20_allowance, build_provider, get_gas_cost, get_received_amount,
        handle_http_response, send_tx_with_retry, TWO_CONFIRMATIONS,
    },
};

pub mod api_types;

// -------------
// | Constants |
// -------------

/// The base URL for the Bebop API
const BEBOP_BASE_URL: &str = "https://api.bebop.xyz/router";

/// The endpoint for getting a quote
const BEBOP_QUOTE_ENDPOINT: &str = "v1/quote";

/// Our Bebop source name
const BEBOP_SOURCE: &str = "renegade";

/// The header to specify the API key for an API request
const BEBOP_API_KEY_HEADER: &str = "source-auth";

// ---------
// | Types |
// ---------

/// Bebop quote execution data
#[derive(Debug, Clone)]
pub struct BebopQuoteExecutionData {
    /// The address of the swap contract
    pub to: Address,
    /// The submitting address
    pub from: Address,
    /// The value of the tx; should be zero
    pub value: U256,
    /// The calldata for the swap
    pub data: Bytes,
    /// The approval target for the swap
    pub approval_target: Address,
}

impl ExecutableQuote {
    /// Convert a Bebop quote into an executable quote for the given source
    pub fn from_bebop_quote(
        bebop_quote: &BebopQuoteResponse,
        bebop_quote_source: CrossVenueQuoteSource,
        chain: Chain,
    ) -> Result<Self, ExecutionClientError> {
        let sell_token = bebop_quote.sell_token(chain, &bebop_quote_source)?;
        let buy_token = bebop_quote.buy_token(chain, &bebop_quote_source)?;
        let sell_amount = bebop_quote.sell_amount(&bebop_quote_source)?;
        let buy_amount = bebop_quote.buy_amount(&bebop_quote_source)?;

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Bebop,
            source: bebop_quote_source.clone(),
            chain,
        };

        let to = bebop_quote.get_to_address(&bebop_quote_source)?;
        let from = bebop_quote.get_from_address(&bebop_quote_source)?;
        let value = bebop_quote.get_value(&bebop_quote_source)?;
        let data = bebop_quote.get_data(&bebop_quote_source)?;
        let approval_target = bebop_quote.get_approval_target(&bebop_quote_source)?;

        let execution_data = BebopQuoteExecutionData { to, from, value, data, approval_target };

        Ok(ExecutableQuote { quote, execution_data: QuoteExecutionData::Bebop(execution_data) })
    }
}

// ----------
// | Client |
// ----------

/// A client for interacting with the Bebop API
#[derive(Clone)]
pub struct BebopClient {
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

impl BebopClient {
    /// Create a new client
    pub fn new(
        api_key: Option<String>,
        base_provider: DynProvider,
        hot_wallet: PrivateKeySigner,
        chain: Chain,
    ) -> Self {
        let hot_wallet_address = hot_wallet.address();
        let rpc_provider = build_provider(base_provider, Some(hot_wallet));

        Self { api_key, http_client: Client::new(), rpc_provider, hot_wallet_address, chain }
    }

    /// Build a Bebop API URL for a given path
    fn build_bebop_url(&self, path: &str) -> Result<String, ExecutionClientError> {
        let chain = to_bebop_chain(self.chain)?;
        let url = format!("{BEBOP_BASE_URL}/{chain}/{path}");
        Ok(url)
    }

    /// Send a get request to Bebop
    async fn send_get_request<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, ExecutionClientError> {
        let url = self.build_bebop_url(path)?;

        let mut request = self.http_client.get(url);
        if let Some(api_key) = &self.api_key {
            request = request.header(BEBOP_API_KEY_HEADER, api_key.as_str());
        }

        let response = request.send().await?;
        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Construct Bebop quote parameters from a venue-agnostic quote params
    /// object, with reasonable defaults.
    fn construct_quote_params(
        &self,
        params: QuoteParams,
    ) -> Result<BebopQuoteParams, ExecutionClientError> {
        let sell_tokens = get_checksummed_address(&params.from_token)?;
        let buy_tokens = get_checksummed_address(&params.to_token)?;

        Ok(BebopQuoteParams {
            sell_tokens,
            buy_tokens,
            sell_amounts: params.from_amount.to_string(),
            taker_address: self.hot_wallet_address.to_string(),
            approval_type: ApprovalType::Standard,
            gasless: false, // We want to self-execute the quote
            slippage: params.slippage_tolerance.unwrap_or(DEFAULT_SLIPPAGE_TOLERANCE),
            // We skip taker checks (approvals, gas estimate, balance) when requesting a quote,
            // as we only want these constraints to be enforced if/when we execute the quote.
            skip_validation: true,
            skip_taker_checks: true,
            source: BEBOP_SOURCE.to_string(),
        })
    }

    /// Approve an erc20 allowance for the given approval target
    #[instrument(skip(self))]
    async fn approve_erc20_allowance(
        &self,
        token_address: Address,
        amount: U256,
        approval_target: Address,
    ) -> Result<(), ExecutionClientError> {
        approve_erc20_allowance(
            token_address,
            approval_target,
            self.hot_wallet_address,
            amount,
            self.rpc_provider.clone(),
        )
        .await
        .map_err(ExecutionClientError::onchain)
    }

    /// Build a swap transaction from Bebop execution data
    async fn build_swap_tx(
        &self,
        execution_data: &BebopQuoteExecutionData,
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

        let tx = TransactionRequest::default()
            .with_to(execution_data.to)
            .with_from(execution_data.from)
            .with_value(execution_data.value)
            .with_input(execution_data.data.clone())
            .with_max_fee_per_gas(latest_basefee * 2)
            .with_max_priority_fee_per_gas(latest_basefee * 2);

        Ok(tx)
    }

    /// Send an onchain transaction with the configured RPC provider
    /// (expected to be configured with a signer)
    async fn send_tx(
        &self,
        tx: TransactionRequest,
    ) -> Result<TransactionReceipt, ExecutionClientError> {
        send_tx_with_retry(tx, &self.rpc_provider, TWO_CONFIRMATIONS)
            .await
            .map_err(ExecutionClientError::onchain)
    }
}

// ------------------------
// | Execution Venue Impl |
// ------------------------

#[async_trait]
impl ExecutionVenue for BebopClient {
    /// Get the name of the venue
    fn venue_specifier(&self) -> SupportedExecutionVenue {
        SupportedExecutionVenue::Bebop
    }

    /// Get a quote from the Bebop API
    #[instrument(skip_all)]
    async fn get_quotes(
        &self,
        params: QuoteParams,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<Vec<ExecutableQuote>, ExecutionClientError> {
        let bebop_quote_sources: Vec<CrossVenueQuoteSource> =
            [CrossVenueQuoteSource::BebopJAMv2, CrossVenueQuoteSource::BebopPMMv3]
                .into_iter()
                .filter(|source| !excluded_quote_sources.contains(source))
                .collect();

        let quote_params = self.construct_quote_params(params)?;
        let query_string =
            serde_qs::to_string(&quote_params).map_err(ExecutionClientError::parse)?;

        let path = format!("{BEBOP_QUOTE_ENDPOINT}?{query_string}");
        let quote_response: BebopQuoteResponse = self.send_get_request(&path).await?;

        let mut executable_quotes = Vec::new();
        for source in bebop_quote_sources {
            match ExecutableQuote::from_bebop_quote(&quote_response, source, self.chain) {
                Ok(executable_quote) => executable_quotes.push(executable_quote),
                Err(e) => warn!("Failed to convert Bebop quote to executable quote: {e}"),
            }
        }

        Ok(executable_quotes)
    }

    /// Execute a quote from the Bebop API
    #[instrument(skip_all)]
    async fn execute_quote(
        &self,
        executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError> {
        let ExecutableQuote { quote, execution_data } = executable_quote;
        let bebop_execution_data = execution_data.bebop()?;

        self.approve_erc20_allowance(
            quote.sell_token.get_alloy_address(),
            quote.sell_amount,
            bebop_execution_data.approval_target,
        )
        .await?;

        let tx = self.build_swap_tx(&bebop_execution_data).await?;

        info!("Executing {} quote", quote.source);

        match self.send_tx(tx).await {
            Ok(receipt) => {
                let gas_cost = get_gas_cost(&receipt);
                let tx_hash = receipt.transaction_hash;

                if receipt.status() {
                    let recipient = bebop_execution_data.from;
                    let buy_token_address = quote.buy_token.get_alloy_address();
                    let buy_amount_actual =
                        get_received_amount(&receipt, buy_token_address, recipient);

                    Ok(ExecutionResult { buy_amount_actual, gas_cost, tx_hash: Some(tx_hash) })
                } else {
                    warn!("tx ({tx_hash:#x}) reverted");
                    Ok(ExecutionResult { buy_amount_actual: U256::ZERO, gas_cost, tx_hash: None })
                }
            },
            Err(e) => {
                warn!("swap tx failed to send: {e}");
                Ok(ExecutionResult {
                    buy_amount_actual: U256::ZERO,
                    gas_cost: U256::ZERO,
                    tx_hash: None,
                })
            },
        }
    }
}

// -----------
// | Helpers |
// -----------

/// Convert a `Chain` to a Bebop chain name
fn to_bebop_chain(chain: Chain) -> Result<String, ExecutionClientError> {
    match chain {
        Chain::ArbitrumOne => Ok("arbitrum".to_string()),
        Chain::BaseMainnet => Ok("base".to_string()),
        _ => Err(ExecutionClientError::onchain(format!("Bebop does not support chain: {chain}"))),
    }
}

/// Get a checksummed address for a given address string
fn get_checksummed_address(address: &str) -> Result<String, ExecutionClientError> {
    Address::from_str(address)
        .map(|addr| addr.to_checksum(None /* chain_id */))
        .map_err(ExecutionClientError::parse)
}
