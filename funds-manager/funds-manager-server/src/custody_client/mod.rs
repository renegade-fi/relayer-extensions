//! Manages the custody backend for the funds manager
#![allow(missing_docs)]
pub mod deposit;
pub mod withdraw;

use ethers::prelude::abigen;
use ethers::providers::{Http, Provider};
use ethers::types::Address;
use fireblocks_sdk::types::Transaction;
use fireblocks_sdk::{
    types::{Account as FireblocksAccount, AccountAsset},
    Client as FireblocksClient, ClientBuilder as FireblocksClientBuilder,
};
use renegade_util::err_str;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::error::FundsManagerError;

abigen!(
    ERC20,
    r#"[
        function symbol() external view returns (string memory)
    ]"#
);

/// The source of a deposit
#[derive(Clone, Copy)]
pub(crate) enum DepositWithdrawSource {
    /// A Renegade quoter deposit or withdrawal
    Quoter,
    /// A fee redemption deposit
    FeeRedemption,
    /// A gas withdrawal
    Gas,
}

impl DepositWithdrawSource {
    /// Get the Fireblocks vault name into which the given deposit source should
    /// deposit funds
    pub(crate) fn get_vault_name(&self) -> &str {
        match self {
            Self::Quoter => "Quoters",
            Self::FeeRedemption => "Fee Collection",
            Self::Gas => "Arbitrum Gas",
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

    /// Get the vault account for a given asset and source
    pub(crate) async fn get_vault_account(
        &self,
        source: &DepositWithdrawSource,
    ) -> Result<Option<FireblocksAccount>, FundsManagerError> {
        let client = self.get_fireblocks_client()?;
        let req = fireblocks_sdk::PagingVaultRequestBuilder::new()
            .limit(100)
            .build()
            .map_err(err_str!(FundsManagerError::Fireblocks))?;

        let (vaults, _rid) = client.vaults(req).await?;
        for vault in vaults.accounts.into_iter() {
            if vault.name == source.get_vault_name() {
                return Ok(Some(vault));
            }
        }

        Ok(None)
    }

    /// Find the wallet in a vault account for a given symbol
    pub(crate) fn get_wallet_for_ticker(
        &self,
        vault: &FireblocksAccount,
        symbol: &str,
    ) -> Option<AccountAsset> {
        vault.assets.iter().find(|acct| acct.id.starts_with(symbol)).cloned()
    }

    /// Poll a fireblocks transaction for completion
    pub(crate) async fn poll_fireblocks_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<Transaction, FundsManagerError> {
        let client = self.get_fireblocks_client()?;
        let timeout = Duration::from_secs(60);
        let interval = Duration::from_secs(5);
        client
            .poll_transaction(transaction_id, timeout, interval, |tx| {
                info!("tx {}: {:?}", transaction_id, tx.status);
            })
            .await
            .map_err(FundsManagerError::fireblocks)
            .map(|(tx, _rid)| tx)
    }
}
