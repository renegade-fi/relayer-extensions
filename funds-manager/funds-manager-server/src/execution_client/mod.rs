//! Client for interacting with execution venues, currently this is the 0x swap
//! API
pub mod error;
pub mod quotes;
pub mod swap;

use std::sync::Arc;

use ethers::{
    middleware::SignerMiddleware,
    providers::{Http, Provider},
    signers::LocalWallet,
};
use http::StatusCode;
use reqwest::{Client, Url};
use serde::Deserialize;
use tracing::error;

use self::error::ExecutionClientError;

/// The 0x api key header
const API_KEY_HEADER: &str = "0x-api-key";

/// The client for interacting with the execution venue
#[derive(Clone)]
pub struct ExecutionClient {
    /// The API key to use for requests
    api_key: String,
    /// The base URL for the execution client
    base_url: String,
    /// The underlying HTTP client
    http_client: Arc<Client>,
    /// The RPC provider
    rpc_provider: Arc<Provider<Http>>,
}

impl ExecutionClient {
    /// Create a new client
    pub fn new(
        api_key: String,
        base_url: String,
        rpc_url: &str,
    ) -> Result<Self, ExecutionClientError> {
        let provider =
            Provider::<Http>::try_from(rpc_url).map_err(ExecutionClientError::arbitrum)?;
        Ok(Self {
            api_key,
            base_url,
            http_client: Arc::new(Client::new()),
            rpc_provider: Arc::new(provider),
        })
    }

    /// Get a full URL for a given endpoint
    fn build_url(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<Url, ExecutionClientError> {
        let url = if !endpoint.starts_with('/') {
            format!("{}/{}", self.base_url, endpoint)
        } else {
            format!("{}{}", self.base_url, endpoint)
        };

        Url::parse_with_params(&url, params).map_err(ExecutionClientError::parse)
    }

    /// Send a get request to the execution venue
    async fn send_get_request<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<T, ExecutionClientError> {
        let url = self.build_url(endpoint, params)?;
        let response =
            self.http_client.get(url).header(API_KEY_HEADER, &self.api_key).send().await?;

        let status = response.status();
        if status != StatusCode::OK {
            let body = response.text().await?;
            let msg = format!("Unexpected status code: {status}\nbody: {body}");
            error!(msg);
            return Err(ExecutionClientError::http(msg));
        }

        response.json::<T>().await.map_err(ExecutionClientError::http)
    }

    /// Get an instance of a signer middleware with the http provider attached
    fn get_signer(
        &self,
        wallet: LocalWallet,
    ) -> SignerMiddleware<Arc<Provider<Http>>, LocalWallet> {
        SignerMiddleware::new(self.rpc_provider.clone(), wallet)
    }
}
