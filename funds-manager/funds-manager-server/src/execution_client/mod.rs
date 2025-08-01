//! Client for interacting with execution venues
pub mod error;
pub mod swap;
pub mod venues;

use alloy::providers::DynProvider;
use price_reporter_client::PriceReporterClient;
use renegade_common::types::chain::Chain;

use crate::helpers::{build_provider, get_erc20_balance};

use self::error::ExecutionClientError;

/// The client for interacting with the execution venue
#[derive(Clone)]
pub struct ExecutionClient {
    /// The chain on which the execution client settles transactions
    chain: Chain,
    /// The RPC provider
    rpc_provider: DynProvider,
    /// The price reporter client
    price_reporter: PriceReporterClient,
}

impl ExecutionClient {
    /// Create a new client
    pub fn new(
        chain: Chain,
        _api_key: Option<String>,
        rpc_url: &str,
        price_reporter: PriceReporterClient,
    ) -> Result<Self, ExecutionClientError> {
        let rpc_provider = build_provider(rpc_url).map_err(ExecutionClientError::parse)?;

        Ok(Self { chain, rpc_provider, price_reporter })
    }

    /// Get the erc20 balance of an address
    pub(crate) async fn get_erc20_balance(
        &self,
        token_address: &str,
        address: &str,
    ) -> Result<f64, ExecutionClientError> {
        get_erc20_balance(token_address, address, self.rpc_provider.clone())
            .await
            .map_err(ExecutionClientError::onchain)
    }
}
