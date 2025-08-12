//! Bebop-specific logic for getting quotes and executing swaps

use std::str::FromStr;

use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
use alloy_primitives::{Address, Bytes, U256};
use async_trait::async_trait;
use funds_manager_api::quoters::{QuoteParams, SupportedExecutionVenue};
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::Deserialize;
use tracing::instrument;

use crate::{
    execution_client::{
        error::ExecutionClientError,
        swap::DEFAULT_SLIPPAGE_TOLERANCE,
        venues::{
            bebop::api_types::{
                ApprovalType, BebopQuoteParams, BebopQuoteResponse, BebopRouteSource,
            },
            quote::{ExecutableQuote, ExecutionQuote, QuoteExecutionData},
            ExecutionResult, ExecutionVenue,
        },
    },
    helpers::{build_provider, handle_http_response},
};

pub mod api_types;

// -------------
// | Constants |
// -------------

/// The base URL for the Bebop API
const BEBOP_BASE_URL: &str = "https://api.bebop.xyz/router";

/// The endpoint for getting a quote
const BEBOP_QUOTE_ENDPOINT: &str = "v1/quote";

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
    /// The gas limit for the swap
    pub gas_limit: U256,
    /// The approval target for the swap
    pub approval_target: Address,
    /// The source of the solution for the quote
    pub route_source: BebopRouteSource,
}

impl ExecutableQuote {
    /// Convert a Bebop quote into an executable quote
    pub fn from_bebop_quote(
        bebop_quote: BebopQuoteResponse,
        chain: Chain,
    ) -> Result<Self, ExecutionClientError> {
        let sell_token = bebop_quote.sell_token(chain)?;
        let buy_token = bebop_quote.buy_token(chain)?;
        let sell_amount = bebop_quote.sell_amount()?;
        let buy_amount = bebop_quote.buy_amount()?;

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Bebop,
            chain,
        };

        let to = bebop_quote.get_to_address()?;
        let from = bebop_quote.get_from_address()?;
        let value = bebop_quote.get_value()?;
        let data = bebop_quote.get_data()?;
        let gas_limit = bebop_quote.get_gas_limit()?;
        let approval_target = bebop_quote.get_approval_target()?;
        let route_source = bebop_quote.get_route_source()?;

        let execution_data = BebopQuoteExecutionData {
            to,
            from,
            value,
            data,
            gas_limit,
            approval_target,
            route_source,
        };

        Ok(ExecutableQuote { quote, execution_data: QuoteExecutionData::Bebop(execution_data) })
    }
}

// ----------
// | Client |
// ----------

/// A client for interacting with the Bebop API
#[derive(Clone)]
pub struct BebopClient {
    /// The underlying HTTP client
    http_client: Client,
    /// The RPC provider
    _rpc_provider: DynProvider,
    /// The address of the hot wallet used for executing quotes
    hot_wallet_address: Address,
    /// The chain on which the client is operating
    chain: Chain,
}

impl BebopClient {
    /// Create a new client
    pub fn new(rpc_url: &str, hot_wallet: PrivateKeySigner, chain: Chain) -> Self {
        let hot_wallet_address = hot_wallet.address();
        let _rpc_provider = build_provider(rpc_url, Some(hot_wallet));

        Self { http_client: Client::new(), _rpc_provider, hot_wallet_address, chain }
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

        let response = self.http_client.get(url).send().await?;
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
        })
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
    async fn get_quote(
        &self,
        params: QuoteParams,
    ) -> Result<ExecutableQuote, ExecutionClientError> {
        let quote_params = self.construct_quote_params(params)?;
        let query_string =
            serde_qs::to_string(&quote_params).map_err(ExecutionClientError::parse)?;

        let path = format!("{BEBOP_QUOTE_ENDPOINT}?{query_string}");
        let quote_response: BebopQuoteResponse = self.send_get_request(&path).await?;
        let executable_quote = ExecutableQuote::from_bebop_quote(quote_response, self.chain)?;

        Ok(executable_quote)
    }

    /// Execute a quote from the Bebop API
    #[instrument(skip_all)]
    async fn execute_quote(
        &self,
        _executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError> {
        todo!()
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
