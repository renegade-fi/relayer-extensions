//! Cowswap-specific logic for getting quotes and executing swaps.

use alloy_primitives::Address;
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{execution_client::error::ExecutionClientError, helpers::handle_http_response};

pub mod api_types;

// -------------
// | Constants |
// -------------

/// The base URL for the Cowswap API
const COWSWAP_BASE_URL: &str = "https://api.cow.fi";

/// The path fragment containing the API version for Cowswap endpoints
const COWSWAP_API_VERSION_PATH_SEGMENT: &str = "api/v1";

// ----------
// | Client |
// ----------

/// A client for interacting with the Cowswap API
#[derive(Clone)]
pub struct CowswapClient {
    /// The underlying HTTP client
    http_client: Client,
    /// The address of the hot wallet used for executing quotes
    hot_wallet_address: Address,
    /// The chain on which the client is operating
    chain: Chain,
}

impl CowswapClient {
    /// Create a new client
    pub fn new(hot_wallet_address: Address, chain: Chain) -> Self {
        Self { http_client: Client::new(), hot_wallet_address, chain }
    }

    /// Send a POST request to the Cowswap API
    async fn send_post_request<Req: Serialize, Res: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: Req,
    ) -> Result<Res, ExecutionClientError> {
        let url = self.build_cowswap_url(path)?;
        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Send a GET request to the Cowswap API
    async fn send_get_request<Res: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<Res, ExecutionClientError> {
        let url = self.build_cowswap_url(path)?;
        let response = self.http_client.get(url).send().await?;

        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Build a Cowswap API URL for a given path
    fn build_cowswap_url(&self, path: &str) -> Result<String, ExecutionClientError> {
        let cowswap_chain = to_cowswap_chain(self.chain)?;
        Ok(format!("{COWSWAP_BASE_URL}/{cowswap_chain}/{COWSWAP_API_VERSION_PATH_SEGMENT}/{path}"))
    }
}

// -----------
// | Helpers |
// -----------

/// Convert a `Chain` to a Cowswap chain name
fn to_cowswap_chain(chain: Chain) -> Result<String, ExecutionClientError> {
    match chain {
        Chain::ArbitrumOne => Ok("arbitrum_one".to_string()),
        Chain::BaseMainnet => Ok("base".to_string()),
        _ => Err(ExecutionClientError::onchain(format!("Cowswap does not support chain: {chain}"))),
    }
}
