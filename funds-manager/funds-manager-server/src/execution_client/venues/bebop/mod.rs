//! Bebop-specific logic for getting quotes and executing swaps

use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
use alloy_primitives::Address;
use async_trait::async_trait;
use funds_manager_api::quoters::{QuoteParams, SupportedExecutionVenue};
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::Deserialize;
use tracing::{info, instrument};

use crate::{
    execution_client::{
        error::ExecutionClientError,
        swap::DEFAULT_SLIPPAGE_TOLERANCE,
        venues::{
            bebop::api_types::{ApprovalType, BebopQuoteParams, BebopQuoteResponse},
            quote::ExecutableQuote,
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
    fn construct_quote_params(&self, params: QuoteParams) -> BebopQuoteParams {
        BebopQuoteParams {
            sell_tokens: params.from_token,
            buy_tokens: params.to_token,
            sell_amounts: params.from_amount.to_string(),
            taker_address: self.hot_wallet_address.to_string(),
            approval_type: ApprovalType::Standard,
            gasless: false, // We want to self-execute the quote
            slippage: params.slippage_tolerance.unwrap_or(DEFAULT_SLIPPAGE_TOLERANCE),
            // We skip taker checks (approvals, gas estimate, balance) when requesting a quote,
            // as we only want these constraints to be enforced if/when we execute the quote.
            skip_validation: true,
            skip_taker_checks: true,
        }
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
        let quote_params = self.construct_quote_params(params);
        let query_string =
            serde_qs::to_string(&quote_params).map_err(ExecutionClientError::parse)?;

        let path = format!("{BEBOP_QUOTE_ENDPOINT}?{query_string}");
        let quote_response: BebopQuoteResponse = self.send_get_request(&path).await?;

        let _executable_quote = match quote_response {
            BebopQuoteResponse::Error(error_response) => {
                return Err(ExecutionClientError::custom(format!(
                    "Error getting Bebop quote: {error_response:?}"
                )))
            },
            BebopQuoteResponse::Successful(quote_response) => {
                info!("Bebop quote response: {quote_response:?}");
                todo!()
            },
        };

        Ok(_executable_quote)
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
