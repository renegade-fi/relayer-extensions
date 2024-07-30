//! Manages the custody backend for the funds manager
#![allow(missing_docs)]
pub mod deposit;
mod hot_wallets;
mod queries;
pub mod withdraw;

use aws_config::SdkConfig as AwsConfig;
use ethers::middleware::SignerMiddleware;
use ethers::prelude::abigen;
use ethers::providers::{Http, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, TransactionReceipt};
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

use crate::db::{DbConn, DbPool};
use crate::error::FundsManagerError;

abigen!(
    ERC20,
    r#"[
        function balanceOf(address account) external view returns (uint256)
        function transfer(address recipient, uint256 amount) external returns (bool)
        function symbol() external view returns (string memory)
        function decimals() external view returns (uint8)
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
    pub(crate) fn vault_name(&self) -> &str {
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
    /// The chain ID
    chain_id: u64,
    /// The API key for the Fireblocks API
    fireblocks_api_key: String,
    /// The API secret for the Fireblocks API
    fireblocks_api_secret: Vec<u8>,
    /// The arbitrum RPC url to use for the custody client
    arbitrum_rpc_url: String,
    /// The database connection pool
    db_pool: Arc<DbPool>,
    /// The AWS config
    aws_config: AwsConfig,
}

impl CustodyClient {
    /// Create a new CustodyClient
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        chain_id: u64,
        fireblocks_api_key: String,
        fireblocks_api_secret: String,
        arbitrum_rpc_url: String,
        db_pool: Arc<DbPool>,
        aws_config: AwsConfig,
    ) -> Self {
        let fireblocks_api_secret = fireblocks_api_secret.as_bytes().to_vec();
        Self {
            chain_id,
            fireblocks_api_key,
            fireblocks_api_secret,
            arbitrum_rpc_url,
            db_pool,
            aws_config,
        }
    }

    /// Get a database connection from the pool
    pub async fn get_db_conn(&self) -> Result<DbConn, FundsManagerError> {
        self.db_pool.get().await.map_err(|e| FundsManagerError::Db(e.to_string()))
    }

    // --- Fireblocks --- //

    /// Get a fireblocks client
    pub fn get_fireblocks_client(&self) -> Result<FireblocksClient, FundsManagerError> {
        FireblocksClientBuilder::new(&self.fireblocks_api_key, &self.fireblocks_api_secret)
            // TODO: Remove the sandbox config
            .with_sandbox()
            .build()
            .map_err(FundsManagerError::fireblocks)
    }

    /// Get the vault account for a given asset and source
    pub(crate) async fn get_vault_account(
        &self,
        name: &str,
    ) -> Result<Option<FireblocksAccount>, FundsManagerError> {
        let client = self.get_fireblocks_client()?;
        let req = fireblocks_sdk::PagingVaultRequestBuilder::new()
            .limit(100)
            .build()
            .map_err(err_str!(FundsManagerError::Fireblocks))?;

        let (vaults, _rid) = client.vaults(req).await?;
        for vault in vaults.accounts.into_iter() {
            if vault.name == name {
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

    // --- Arbitrum JSON RPC --- //

    /// Get a JSON RPC provider for the given RPC url
    pub fn get_rpc_provider(&self) -> Result<Provider<Http>, FundsManagerError> {
        Provider::<Http>::try_from(&self.arbitrum_rpc_url)
            .map_err(err_str!(FundsManagerError::Arbitrum))
    }

    /// Get the symbol for an ERC20 token at the given address
    pub(self) async fn get_erc20_token_symbol(
        &self,
        token_address: &str,
    ) -> Result<String, FundsManagerError> {
        let addr =
            Address::from_str(token_address).map_err(err_str!(FundsManagerError::Arbitrum))?;
        let provider = self.get_rpc_provider()?;
        let client = Arc::new(provider);
        let erc20 = ERC20::new(addr, client);

        erc20.symbol().call().await.map_err(FundsManagerError::arbitrum)
    }

    /// Perform an erc20 transfer
    pub(crate) async fn erc20_transfer(
        &self,
        mint: &str,
        to_address: &str,
        amount: f64,
        wallet: LocalWallet,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        // Set the chain ID
        let wallet = wallet.with_chain_id(self.chain_id);

        // Setup the provider
        let provider = self.get_rpc_provider()?;
        let client = SignerMiddleware::new(provider, wallet);
        let token_address = Address::from_str(mint).map_err(FundsManagerError::parse)?;
        let token = ERC20::new(token_address, Arc::new(client));

        // Convert the amount using the token's decimals
        let decimals = token.decimals().call().await.map_err(FundsManagerError::arbitrum)? as u32;
        let amount = ethers::utils::parse_units(amount.to_string(), decimals)
            .map_err(FundsManagerError::parse)?
            .into();

        // Transfer the tokens
        let to_address = Address::from_str(to_address).map_err(FundsManagerError::parse)?;
        let tx = token.transfer(to_address, amount);
        let pending_tx = tx.send().await.map_err(|e| {
            FundsManagerError::arbitrum(format!("Failed to send transaction: {}", e))
        })?;

        pending_tx
            .await
            .map_err(FundsManagerError::arbitrum)?
            .ok_or_else(|| FundsManagerError::arbitrum("Transaction failed".to_string()))
    }
}
