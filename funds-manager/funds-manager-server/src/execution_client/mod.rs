//! Client for interacting with execution venues
pub mod error;
pub mod swap;
pub mod venues;

use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
use alloy_primitives::Address;
use price_reporter_client::PriceReporterClient;
use renegade_common::types::chain::Chain;

use crate::{
    execution_client::venues::{lifi::LifiClient, AllExecutionVenues},
    helpers::{build_provider, get_erc20_balance},
};

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
    /// The address of the hot wallet used for executing quotes
    hot_wallet_address: Address,
    /// The venues used for execution
    venues: AllExecutionVenues,
}

impl ExecutionClient {
    /// Create a new client
    pub fn new(
        chain: Chain,
        lifi_api_key: Option<String>,
        rpc_url: &str,
        price_reporter: PriceReporterClient,
        quoter_hot_wallet: PrivateKeySigner,
    ) -> Result<Self, ExecutionClientError> {
        let hot_wallet_address = quoter_hot_wallet.address();
        let rpc_provider = build_provider(rpc_url).map_err(ExecutionClientError::parse)?;

        let lifi = LifiClient::new(lifi_api_key, rpc_provider.clone(), quoter_hot_wallet, chain)?;
        let venues = AllExecutionVenues { lifi };

        Ok(Self { chain, rpc_provider, price_reporter, hot_wallet_address, venues })
    }

    /// Get the erc20 balance of an address
    pub(crate) async fn get_erc20_balance(
        &self,
        token_address: &str,
    ) -> Result<f64, ExecutionClientError> {
        get_erc20_balance(
            token_address,
            &self.hot_wallet_address.to_string(),
            self.rpc_provider.clone(),
        )
        .await
        .map_err(ExecutionClientError::onchain)
    }
}
