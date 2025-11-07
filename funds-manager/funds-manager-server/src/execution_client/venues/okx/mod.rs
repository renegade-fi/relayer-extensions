//! Okx-specific logic for getting quotes and executing swaps

use std::{collections::HashMap, iter, str::FromStr};

use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
use alloy_primitives::{Address, Bytes, U256};
use async_trait::async_trait;
use base64::Engine;
use chrono::{SecondsFormat, Utc};
use funds_manager_api::quoters::{QuoteParams, SupportedExecutionVenue};
use hmac::Mac;
use itertools::Itertools;
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::Deserialize;

use crate::{
    execution_client::{
        error::ExecutionClientError,
        swap::DEFAULT_SLIPPAGE_TOLERANCE,
        venues::{
            okx::api_types::{
                OkxApiResponse, OkxApproveRequestParams, OkxApproveResponse, OkxLiquiditySource,
                OkxSwapRequestParams, OkxSwapResponse,
            },
            quote::{CrossVenueQuoteSource, ExecutableQuote, ExecutionQuote, QuoteExecutionData},
            ExecutionResult, ExecutionVenue,
        },
    },
    helpers::{build_provider, handle_http_response, to_chain_id},
};

pub mod api_types;

// -------------
// | Constants |
// -------------

/// The base URL for the Okx DEX aggregator API
const OKX_BASE_URL: &str = "https://web3.okx.com";

/// The header name for the project ID
const OKX_PROJECT_HEADER: &str = "OK-ACCESS-PROJECT";
/// The header name for the API key
const OKX_API_KEY_HEADER: &str = "OK-ACCESS-KEY";
/// The header name for the API authentication HMAC
const OKX_API_HMAC_HEADER: &str = "OK-ACCESS-SIGN";
/// The header name for the passphrase
const OKX_PASSPHRASE_HEADER: &str = "OK-ACCESS-PASSPHRASE";
/// The header name for the request timestamp
const OKX_TIMESTAMP_HEADER: &str = "OK-ACCESS-TIMESTAMP";

/// The string representation of the HTTP GET method as expected in the Okx API
/// HMAC signature
const HTTP_GET_METHOD: &str = "GET";

/// The endpoint for getting the list of all supported liquidity sources
const OKX_LIQUIDITY_SOURCES_ENDPOINT: &str = "/api/v5/dex/aggregator/get-liquidity";
/// The endpoint for getting a swap payload
const OKX_SWAP_ENDPOINT: &str = "/api/v5/dex/aggregator/swap";
/// The endpoint for getting an approval target for a swap
const OKX_APPROVAL_ENDPOINT: &str = "/api/v5/dex/aggregator/approve-transaction";

/// The query parameter for the chain ID
const OKX_CHAIN_ID_QUERY_PARAM: &str = "chainId";

/// The Renegade DEX name in Okx
///
/// Note: For now this is a placeholder value
const RENEGADE_DEX_NAME: &str = "Renegade";

// ---------
// | Types |
// ---------

/// The credentials required for authenticating with the Okx API
#[derive(Deserialize, Clone)]
pub struct OkxApiCredentials {
    /// The API key to use for requests
    api_key: String,
    /// The secret w/ which to compute request HMACs
    api_secret: String,
    /// The passphrase used to create the API key
    passphrase: String,
    /// The project ID under which the API key was created
    project_id: String,
}

/// Okx quote execution data
#[derive(Debug, Clone)]
pub struct OkxQuoteExecutionData {
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

// ----------
// | Client |
// ----------

/// A client for interacting with the Okx API
#[derive(Clone)]
pub struct OkxClient {
    /// The credentials required for authenticating with the Okx API
    credentials: OkxApiCredentials,
    /// The underlying HTTP client
    http_client: Client,
    /// The RPC provider
    _rpc_provider: DynProvider,
    /// The address of the hot wallet used for executing quotes
    hot_wallet_address: Address,
    /// The chain on which the client is operating
    chain: Chain,
    /// A mapping of liquidity source name -> ID for all supported Okx liquidity
    /// sources
    all_liquidity_sources: HashMap<String, String>,
}

impl OkxClient {
    /// Create a new client
    pub async fn new(
        credentials: OkxApiCredentials,
        base_provider: DynProvider,
        hot_wallet: PrivateKeySigner,
        chain: Chain,
    ) -> Result<Self, ExecutionClientError> {
        let hot_wallet_address = hot_wallet.address();
        let _rpc_provider = build_provider(base_provider, Some(hot_wallet));

        let mut client = Self {
            credentials,
            http_client: Client::new(),
            _rpc_provider,
            hot_wallet_address,
            chain,
            all_liquidity_sources: HashMap::new(),
        };

        client.store_all_liquidity_sources().await?;

        Ok(client)
    }

    /// Send a GET request to the Okx API
    async fn send_get_request<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<T, ExecutionClientError> {
        let url = format!("{OKX_BASE_URL}{path}");

        let iso_timestamp =
            Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true /* use_z */);

        let hmac = self.sign_get_request(&iso_timestamp, path)?;

        let request = self
            .http_client
            .get(url)
            .header(OKX_PROJECT_HEADER, &self.credentials.project_id)
            .header(OKX_API_KEY_HEADER, &self.credentials.api_key)
            .header(OKX_API_HMAC_HEADER, hmac)
            .header(OKX_PASSPHRASE_HEADER, &self.credentials.passphrase)
            .header(OKX_TIMESTAMP_HEADER, iso_timestamp);

        let response = request.send().await?;

        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Create an HMAC signature for a Okx API GET request
    fn sign_get_request(
        &self,
        iso_timestamp: &str,
        path: &str,
    ) -> Result<String, ExecutionClientError> {
        let mut hmac =
            hmac::Hmac::<sha2::Sha256>::new_from_slice(self.credentials.api_secret.as_bytes())
                .map_err(ExecutionClientError::parse)?;

        let message = format!("{iso_timestamp}{HTTP_GET_METHOD}{path}");

        hmac.update(message.as_bytes());

        let hmac_bytes = hmac.finalize().into_bytes();

        Ok(base64::engine::general_purpose::STANDARD.encode(hmac_bytes))
    }

    /// Get the list of all supported liquidity sources
    async fn store_all_liquidity_sources(&mut self) -> Result<(), ExecutionClientError> {
        let chain_id = to_chain_id(self.chain);
        let path =
            format!("{OKX_LIQUIDITY_SOURCES_ENDPOINT}?{OKX_CHAIN_ID_QUERY_PARAM}={chain_id}");

        let response: OkxApiResponse<Vec<OkxLiquiditySource>> =
            self.send_get_request(&path).await?;

        let sources = HashMap::from_iter(response.data.into_iter().map(|s| (s.name, s.id)));
        self.all_liquidity_sources = sources;

        Ok(())
    }

    /// Get the approval target for a swap
    async fn get_approval_target(
        &self,
        token_contract_address: String,
    ) -> Result<Address, ExecutionClientError> {
        let request_params = OkxApproveRequestParams {
            chain_id: to_chain_id(self.chain).to_string(),
            token_contract_address,
            // The approval amount is irrelevant, we just need to get the approval target address
            approve_amount: U256::ONE.to_string(),
        };

        let query_string =
            serde_qs::to_string(&request_params).map_err(ExecutionClientError::parse)?;

        let path = format!("{OKX_APPROVAL_ENDPOINT}?{query_string}");
        let response: OkxApiResponse<[OkxApproveResponse; 1]> =
            self.send_get_request(&path).await?;

        let [approval_response] = response.data;

        Address::from_str(&approval_response.dex_contract_address)
            .map_err(ExecutionClientError::parse)
    }

    /// Construct the parameters for an Okx swap request from the venue-agnostic
    /// QuoteParams & the list of liquidity sources to exclude
    fn construct_swap_request_params(
        &self,
        params: QuoteParams,
        excluded_liquidity_sources: Vec<String>,
    ) -> OkxSwapRequestParams {
        let chain_id = to_chain_id(self.chain).to_string();
        let amount = params.from_amount.to_string();
        let slippage = params.slippage_tolerance.unwrap_or(DEFAULT_SLIPPAGE_TOLERANCE).to_string();
        let user_wallet_address = self.hot_wallet_address.to_string();

        let dex_ids = self
            .all_liquidity_sources
            .iter()
            .filter_map(|(name, id)| (!excluded_liquidity_sources.contains(name)).then_some(id))
            .join(",");

        OkxSwapRequestParams {
            chain_id,
            amount,
            from_token_address: params.from_token,
            to_token_address: params.to_token,
            slippage,
            user_wallet_address,
            dex_ids,
        }
    }
}

// ------------------------
// | Execution Venue Impl |
// ------------------------

#[async_trait]
impl ExecutionVenue for OkxClient {
    fn venue_specifier(&self) -> SupportedExecutionVenue {
        SupportedExecutionVenue::Okx
    }

    async fn get_quotes(
        &self,
        params: QuoteParams,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<Vec<ExecutableQuote>, ExecutionClientError> {
        let excluded_liquidity_sources: Vec<String> = excluded_quote_sources
            .iter()
            .filter_map(|s| match s {
                CrossVenueQuoteSource::Okx(route) => Some(route.clone()),
                _ => None,
            })
            .flatten()
            .chain(iter::once(RENEGADE_DEX_NAME.to_string()))
            .collect();

        let swap_request_params =
            self.construct_swap_request_params(params, excluded_liquidity_sources);

        let query_string =
            serde_qs::to_string(&swap_request_params).map_err(ExecutionClientError::parse)?;

        let swap_api_response: OkxApiResponse<[OkxSwapResponse; 1]> =
            self.send_get_request(&format!("{OKX_SWAP_ENDPOINT}?{query_string}")).await?;

        let [swap_response] = swap_api_response.data;

        let approval_target = self.get_approval_target(swap_response.sell_token_address()).await?;

        let executable_quote =
            ExecutableQuote::from_okx_swap_response(swap_response, approval_target).await?;

        Ok(vec![executable_quote])
    }

    async fn execute_quote(
        &self,
        _executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError> {
        todo!()
    }
}

// -------------------------
// | Executable Quote Impl |
// -------------------------

impl ExecutableQuote {
    /// Convert an Okx swap response into an executable quote
    pub async fn from_okx_swap_response(
        swap_response: OkxSwapResponse,
        approval_target: Address,
    ) -> Result<Self, ExecutionClientError> {
        let sell_token = swap_response.sell_token()?;
        let buy_token = swap_response.buy_token()?;

        let sell_amount = swap_response.sell_amount();
        let buy_amount = swap_response.buy_amount();

        let source = swap_response.quote_source();
        let chain = swap_response.chain()?;

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Okx,
            source,
            chain,
        };

        let to = swap_response.get_to_address()?;
        let from = swap_response.get_from_address()?;
        let value = swap_response.get_value()?;
        let data = swap_response.get_data()?;

        let execution_data = OkxQuoteExecutionData { to, from, value, data, approval_target };

        Ok(ExecutableQuote { quote, execution_data: QuoteExecutionData::Okx(execution_data) })
    }
}
