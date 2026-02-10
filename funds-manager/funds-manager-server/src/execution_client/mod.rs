//! Client for interacting with execution venues
pub mod error;
pub mod swap;
pub mod venues;

use alloy::{providers::DynProvider, signers::local::PrivateKeySigner};
use alloy_primitives::{Address, U256};
use price_reporter_client::PriceReporterClient;
use renegade_types_core::Chain;

use crate::{
    cli::MaxPriceDeviations,
    execution_client::venues::{
        AllExecutionVenues, bebop::BebopClient, cowswap::CowswapClient, lifi::LifiClient,
    },
    helpers::{build_provider, get_erc20_balance, get_erc20_balance_raw},
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
    /// Map from ticker -> max price deviation allowed in a quote for that token
    max_price_deviations: MaxPriceDeviations,
}

impl ExecutionClient {
    /// Create a new client
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        chain: Chain,
        lifi_api_key: Option<String>,
        bebop_api_key: Option<String>,
        base_provider: DynProvider,
        price_reporter: PriceReporterClient,
        quoter_hot_wallet: PrivateKeySigner,
        max_price_deviations: MaxPriceDeviations,
    ) -> Result<Self, ExecutionClientError> {
        let hot_wallet_address = quoter_hot_wallet.address();

        let rpc_provider = build_provider(base_provider.clone(), None /* wallet */);

        let lifi =
            LifiClient::new(lifi_api_key, base_provider.clone(), quoter_hot_wallet.clone(), chain);

        let cowswap = CowswapClient::new(base_provider.clone(), quoter_hot_wallet.clone(), chain);
        let bebop = BebopClient::new(
            bebop_api_key,
            base_provider.clone(),
            quoter_hot_wallet.clone(),
            chain,
        );

        let venues = AllExecutionVenues { lifi, cowswap, bebop };

        Ok(Self {
            chain,
            rpc_provider,
            price_reporter,
            hot_wallet_address,
            venues,
            max_price_deviations,
        })
    }

    /// Get the erc20 balance of an address, as a U256
    pub(crate) async fn get_erc20_balance_raw(
        &self,
        token_address: &str,
    ) -> Result<U256, ExecutionClientError> {
        get_erc20_balance_raw(
            token_address,
            &self.hot_wallet_address.to_string(),
            self.rpc_provider.clone(),
        )
        .await
        .map_err(ExecutionClientError::onchain)
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
