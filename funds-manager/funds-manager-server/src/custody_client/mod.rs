//! Manages the custody backend for the funds manager
pub mod deposit;
pub mod gas_sponsor;
pub mod gas_wallets;
mod hot_wallets;
mod queries;
pub mod withdraw;

use aws_config::SdkConfig as AwsConfig;
use ethers::middleware::SignerMiddleware;
use ethers::providers::{Http, Middleware, Provider};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, TransactionReceipt, TransactionRequest};
use ethers::utils::format_units;
use fireblocks_sdk::types::Transaction;
use fireblocks_sdk::{
    types::Account as FireblocksAccount, Client as FireblocksClient,
    ClientBuilder as FireblocksClientBuilder,
};
use renegade_arbitrum_client::constants::Chain;
use renegade_util::err_str;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use crate::db::{DbConn, DbPool};
use crate::error::FundsManagerError;
use crate::helpers::ERC20;

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

    /// Build a `DepositWithdrawSource` from a vault name
    pub fn from_vault_name(name: &str) -> Result<Self, FundsManagerError> {
        match name.to_lowercase().as_str() {
            "quoters" => Ok(Self::Quoter),
            "fee collection" => Ok(Self::FeeRedemption),
            "arbitrum gas" => Ok(Self::Gas),
            _ => Err(FundsManagerError::parse(format!("invalid vault name: {name}"))),
        }
    }
}

/// The client interacting with the custody backend
#[derive(Clone)]
pub struct CustodyClient {
    /// The chain name
    chain: Chain,
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
    /// The gas sponsor contract address
    gas_sponsor_address: Address,
}

impl CustodyClient {
    /// Create a new CustodyClient
    #[allow(clippy::needless_pass_by_value)]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain: Chain,
        chain_id: u64,
        fireblocks_api_key: String,
        fireblocks_api_secret: String,
        arbitrum_rpc_url: String,
        db_pool: Arc<DbPool>,
        aws_config: AwsConfig,
        gas_sponsor_address: Address,
    ) -> Self {
        let fireblocks_api_secret = fireblocks_api_secret.as_bytes().to_vec();
        Self {
            chain,
            chain_id,
            fireblocks_api_key,
            fireblocks_api_secret,
            arbitrum_rpc_url,
            db_pool,
            aws_config,
            gas_sponsor_address,
        }
    }

    /// Get a database connection from the pool
    pub async fn get_db_conn(&self) -> Result<DbConn, FundsManagerError> {
        self.db_pool.get().await.map_err(|e| FundsManagerError::Db(e.to_string()))
    }

    /// Get the gas sponsor contract address as a string
    pub fn gas_sponsor_address(&self) -> String {
        format!("{:#x}", self.gas_sponsor_address)
    }

    // --- Fireblocks --- //

    /// Get a fireblocks client
    pub fn get_fireblocks_client(&self) -> Result<FireblocksClient, FundsManagerError> {
        FireblocksClientBuilder::new(&self.fireblocks_api_key, &self.fireblocks_api_secret)
            .build()
            .map_err(FundsManagerError::fireblocks)
    }

    /// Get the fireblocks asset ID for a given ERC20 address
    pub(crate) async fn get_asset_id_for_address(
        &self,
        address: &str,
    ) -> Result<Option<String>, FundsManagerError> {
        let client = self.get_fireblocks_client()?;
        let (supported_assets, _rid) = client.supported_assets().await?;
        for asset in supported_assets {
            if asset.contract_address.to_lowercase() == address.to_lowercase() {
                return Ok(Some(asset.id.to_string()));
            }
        }

        Ok(None)
    }

    /// Get the vault account for a given asset and source
    pub(crate) async fn get_vault_account(
        &self,
        name: &str,
    ) -> Result<Option<FireblocksAccount>, FundsManagerError> {
        let client = self.get_fireblocks_client()?;
        let req = fireblocks_sdk::PagingVaultRequestBuilder::new()
            .name_prefix(name)
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

    /// Get the native token balance of an address
    pub(crate) async fn get_ether_balance(&self, address: &str) -> Result<f64, FundsManagerError> {
        let provider = self.get_rpc_provider()?;
        let client = Arc::new(provider);
        let address = Address::from_str(address).map_err(FundsManagerError::parse)?;
        let balance =
            client.get_balance(address, None).await.map_err(FundsManagerError::arbitrum)?;

        // Convert U256 to f64
        let balance_str = format_units(balance, "ether").map_err(FundsManagerError::parse)?;
        balance_str.parse::<f64>().map_err(FundsManagerError::parse)
    }

    /// Transfer ether from the given wallet
    pub(crate) async fn transfer_ether(
        &self,
        to: &str,
        amount: f64,
        wallet: LocalWallet,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        let wallet = wallet.with_chain_id(self.chain_id);
        let provider = self.get_rpc_provider()?;
        let client = SignerMiddleware::new(provider, wallet);

        let to = Address::from_str(to).map_err(FundsManagerError::parse)?;
        let amount_units = ethers::utils::parse_units(amount.to_string(), "ether")
            .map_err(FundsManagerError::parse)?;

        info!("Transferring {amount} ETH to {to:#x}");
        let tx = TransactionRequest::new().to(to).value(amount_units);
        let pending_tx =
            client.send_transaction(tx, None).await.map_err(FundsManagerError::arbitrum)?;
        pending_tx
            .await
            .map_err(FundsManagerError::arbitrum)?
            .ok_or_else(|| FundsManagerError::arbitrum("Transaction failed".to_string()))
    }

    /// Get the erc20 balance of an address
    pub(crate) async fn get_erc20_balance(
        &self,
        token_address: &str,
        address: &str,
    ) -> Result<f64, FundsManagerError> {
        // Setup the provider
        let token_address = Address::from_str(token_address).map_err(FundsManagerError::parse)?;
        let address = Address::from_str(address).map_err(FundsManagerError::parse)?;
        let provider = self.get_rpc_provider()?;
        let client = Arc::new(provider);
        let erc20 = ERC20::new(token_address, client);

        // Fetch the balance and correct for the ERC20 decimal precision
        let decimals = erc20.decimals().call().await.map_err(FundsManagerError::arbitrum)? as u32;
        let balance =
            erc20.balance_of(address).call().await.map_err(FundsManagerError::arbitrum)?;
        let bal_str = format_units(balance, decimals).map_err(FundsManagerError::parse)?;
        let bal_f64 = bal_str.parse::<f64>().map_err(FundsManagerError::parse)?;

        Ok(bal_f64)
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
