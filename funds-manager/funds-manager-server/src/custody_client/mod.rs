//! Manages the custody backend for the funds manager
#![allow(missing_docs)]
pub mod deposit;

use ethers::prelude::abigen;
use ethers::providers::{Http, Provider};
use ethers::types::Address;
use fireblocks_sdk::{Client as FireblocksClient, ClientBuilder as FireblocksClientBuilder};
use renegade_util::err_str;
use std::str::FromStr;
use std::sync::Arc;

use crate::error::FundsManagerError;

abigen!(
    ERC20,
    r#"[
        function symbol() external view returns (string memory)
    ]"#
);

/// The source of a deposit
pub(crate) enum DepositSource {
    /// A Renegade quoter
    Quoter,
    /// A fee withdrawal
    FeeWithdrawal,
}

impl DepositSource {
    /// Get the Fireblocks vault name into which the given deposit source should
    /// deposit funds
    pub(crate) fn get_vault_name(&self) -> &str {
        match self {
            DepositSource::Quoter => "Quoters",
            DepositSource::FeeWithdrawal => unimplemented!("no vault for fee withdrawal yet"),
        }
    }
}

/// The client interacting with the custody backend
#[derive(Clone)]
pub struct CustodyClient {
    /// The API key for the Fireblocks API
    fireblocks_api_key: String,
    /// The API secret for the Fireblocks API
    fireblocks_api_secret: Vec<u8>,
    /// The arbitrum RPC url to use for the custody client
    arbitrum_rpc_url: String,
}

impl CustodyClient {
    /// Create a new CustodyClient
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        fireblocks_api_key: String,
        fireblocks_api_secret: String,
        arbitrum_rpc_url: String,
    ) -> Self {
        let fireblocks_api_secret = fireblocks_api_secret.as_bytes().to_vec();
        Self { fireblocks_api_key, fireblocks_api_secret, arbitrum_rpc_url }
    }

    /// Get a fireblocks client
    pub fn get_fireblocks_client(&self) -> Result<FireblocksClient, FundsManagerError> {
        FireblocksClientBuilder::new(&self.fireblocks_api_key, &self.fireblocks_api_secret)
            // TODO: Remove the sandbox config
            .with_sandbox()
            .build()
            .map_err(FundsManagerError::fireblocks)
    }

    /// Get the symbol for an ERC20 token at the given address
    pub(self) async fn get_erc20_token_symbol(
        &self,
        token_address: &str,
    ) -> Result<String, FundsManagerError> {
        let addr =
            Address::from_str(token_address).map_err(err_str!(FundsManagerError::Arbitrum))?;
        let provider = Provider::<Http>::try_from(&self.arbitrum_rpc_url)
            .map_err(err_str!(FundsManagerError::Arbitrum))?;
        let client = Arc::new(provider);
        let erc20 = ERC20::new(addr, client);

        erc20.symbol().call().await.map_err(FundsManagerError::arbitrum)
    }
}
