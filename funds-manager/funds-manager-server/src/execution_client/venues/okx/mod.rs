//! Okx-specific logic for getting quotes and executing swaps

use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
use base64::Engine;
use chrono::{SecondsFormat, Utc};
use hmac::Mac;
use renegade_common::types::{chain::Chain, hmac::HmacKey};
use reqwest::Client;
use serde::Deserialize;

use crate::{
    execution_client::{
        error::ExecutionClientError, venues::okx::api_types::OkxLiquiditySourcesResponse,
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

/// The query parameter for the chain ID
const OKX_CHAIN_ID_QUERY_PARAM: &str = "chainId";

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
    rpc_provider: DynProvider,
    /// The chain on which the client is operating
    chain: Chain,
    /// The list of all supported liquidity sources, cached here for performance
    all_liquidity_sources: Vec<String>,
}

impl OkxClient {
    /// Create a new client
    pub fn new(
        credentials: OkxApiCredentials,
        base_provider: DynProvider,
        hot_wallet: PrivateKeySigner,
        chain: Chain,
    ) -> Self {
        let rpc_provider = build_provider(base_provider, Some(hot_wallet));

        Self {
            credentials,
            http_client: Client::new(),
            rpc_provider,
            chain,
            all_liquidity_sources: vec![],
        }
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

        let response: OkxLiquiditySourcesResponse = self.send_get_request(&path).await?;

        let source_ids = response.data.into_iter().map(|s| s.id).collect();
        self.all_liquidity_sources = source_ids;

        Ok(())
    }
}
