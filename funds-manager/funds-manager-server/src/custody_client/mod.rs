//! Manages the custody backend for the funds manager
pub mod deposit;
mod fireblocks_client;
pub mod gas_sponsor;
pub mod gas_wallets;
mod hot_wallets;
mod queries;
pub mod rpc_shim;
pub mod vaults;
pub mod withdraw;

use alloy::{
    network::TransactionBuilder,
    providers::{DynProvider, Provider},
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{
    utils::{format_units, parse_units},
    Address, TxHash,
};
use aws_config::SdkConfig as AwsConfig;
use fireblocks_sdk::{
    apis::{
        blockchains_assets_beta_api::{ListAssetsParams, ListBlockchainsParams},
        transactions_api::GetTransactionsParams,
        Api,
    },
    models::TransactionResponse,
};
use price_reporter_client::PriceReporterClient;
use renegade_common::types::chain::Chain;
use std::sync::Arc;
use std::time::Duration;
use std::{str::FromStr, time::Instant};
use tracing::{debug, info};

use crate::{
    custody_client::fireblocks_client::FireblocksClient,
    helpers::{
        get_erc20_balance, send_tx_with_retry, to_env_agnostic_name, IERC20, ONE_CONFIRMATION,
    },
};
use crate::{
    db::{DbConn, DbPool},
    helpers::build_provider,
};
use crate::{error::FundsManagerError, helpers::titlecase};

// -------------
// | Constants |
// -------------

/// The Fireblocks asset ID for ETH on Arbitrum One
const ARB_ONE_ETH_ASSET_ID: &str = "ETH-AETH";
/// The Fireblocks asset ID for ETH on Arbitrum Sepolia
const ARB_SEPOLIA_ETH_ASSET_ID: &str = "ETH-AETH_SEPOLIA";
/// The Fireblocks asset ID for ETH on Base mainnet
const BASE_MAINNET_ETH_ASSET_ID: &str = "BASECHAIN_ETH";
/// The Fireblocks asset ID for ETH on Base Sepolia
const BASE_SEPOLIA_ETH_ASSET_ID: &str = "BASECHAIN_ETH_TEST5";

/// The Fireblocks asset IDs for native assets on testnets
pub const TESTNET_NATIVE_ASSET_IDS: &[&str] =
    &[ARB_SEPOLIA_ETH_ASSET_ID, BASE_SEPOLIA_ETH_ASSET_ID, "ETH_TEST5"];

/// The Fireblocks asset IDs for native assets on mainnets
pub const MAINNET_NATIVE_ASSET_IDS: &[&str] =
    &[ARB_ONE_ETH_ASSET_ID, BASE_MAINNET_ETH_ASSET_ID, "ETH"];

/// The number of confirmations Fireblocks requires to consider a contract call
/// final
const FB_CONTRACT_CONFIRMATIONS: u64 = 3;

/// The error message emitted when an unsupported chain is configured.
const ERR_UNSUPPORTED_CHAIN: &str = "Unsupported chain";
/// The error message for when the Arbitrum blockchain is not found
/// in the Fireblocks `/blockchains` endpoint response
const ERR_ARB_CHAIN_NOT_FOUND: &str = "Arbitrum blockchain not found";

// ---------
// | Types |
// ---------

/// The source of a deposit
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub(crate) fn vault_name(&self, chain: Chain) -> String {
        let env_name = titlecase(&to_env_agnostic_name(chain));
        match self {
            Self::Quoter => format!("{env_name} Quoters"),
            Self::FeeRedemption => format!("{env_name} Fee Collection"),
            Self::Gas => format!("{env_name} Gas"),
        }
    }

    /// Build a `DepositWithdrawSource` from a vault name
    pub fn from_vault_name(name: &str, chain: Chain) -> Result<Self, FundsManagerError> {
        let env_name = to_env_agnostic_name(chain);
        let full_name = format!("{env_name} {name}").to_lowercase();
        match full_name.to_lowercase().as_str() {
            "arbitrum quoters" | "base quoters" => Ok(Self::Quoter),
            "arbitrum fee collection" | "base fee collection" => Ok(Self::FeeRedemption),
            "arbitrum gas" | "base gas" => Ok(Self::Gas),
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
    /// The Fireblocks API client
    fireblocks_client: Arc<FireblocksClient>,
    /// The RPC URL to use for the custody client
    rpc_url: String,
    /// The database connection pool
    db_pool: Arc<DbPool>,
    /// The AWS config
    aws_config: AwsConfig,
    /// The gas sponsor contract address
    gas_sponsor_address: Address,
    /// The price reporter client
    price_reporter: PriceReporterClient,
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
        rpc_url: String,
        db_pool: Arc<DbPool>,
        aws_config: AwsConfig,
        gas_sponsor_address: Address,
        price_reporter: PriceReporterClient,
    ) -> Result<Self, FundsManagerError> {
        let fireblocks_client =
            Arc::new(FireblocksClient::new(&fireblocks_api_key, &fireblocks_api_secret)?);

        Ok(Self {
            chain,
            chain_id,
            fireblocks_client,
            rpc_url,
            db_pool,
            aws_config,
            gas_sponsor_address,
            price_reporter,
        })
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

    /// Get the fireblocks asset ID for a given ERC20 address
    pub(crate) async fn get_asset_id_for_address(
        &self,
        address: &str,
    ) -> Result<Option<String>, FundsManagerError> {
        if let Some(asset_id) = self.fireblocks_client.read_cached_asset_id(address).await {
            return Ok(Some(asset_id));
        }

        let blockchain_id = self.get_current_blockchain_id().await?;
        let list_assets_params =
            ListAssetsParams::builder().blockchain_id(blockchain_id).page_size(1000.0).build();

        let arb_assets = self
            .fireblocks_client
            .sdk
            .apis()
            .blockchains_assets_beta_api()
            .list_assets(list_assets_params)
            .await?;

        for asset in arb_assets.data {
            if let Some(contract_address) = asset.onchain.and_then(|o| o.address) {
                if contract_address.to_lowercase() == address.to_lowercase() {
                    let asset_id = asset.legacy_id;

                    self.fireblocks_client
                        .cache_asset_id(address.to_string(), asset_id.clone())
                        .await;

                    return Ok(Some(asset_id));
                }
            }
        }

        Ok(None)
    }

    /// Get the Fireblocks asset ID for the native asset (ETH) of the configured
    /// chain.
    pub(crate) fn get_native_eth_asset_id(&self) -> Result<String, FundsManagerError> {
        match self.chain {
            Chain::ArbitrumOne => Ok(ARB_ONE_ETH_ASSET_ID.to_string()),
            Chain::ArbitrumSepolia => Ok(ARB_SEPOLIA_ETH_ASSET_ID.to_string()),
            Chain::BaseMainnet => Ok(BASE_MAINNET_ETH_ASSET_ID.to_string()),
            Chain::BaseSepolia => Ok(BASE_SEPOLIA_ETH_ASSET_ID.to_string()),
            _ => Err(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN)),
        }
    }

    /// Get the Fireblocks asset IDs for native assets on the current chain
    pub(crate) fn get_current_env_native_asset_ids(&self) -> Result<&[&str], FundsManagerError> {
        match self.chain {
            Chain::ArbitrumOne | Chain::BaseMainnet => Ok(MAINNET_NATIVE_ASSET_IDS),
            Chain::ArbitrumSepolia | Chain::BaseSepolia => Ok(TESTNET_NATIVE_ASSET_IDS),
            _ => Err(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN)),
        }
    }

    /// Get the Fireblocks blockchain ID for the current chain
    async fn get_current_blockchain_id(&self) -> Result<String, FundsManagerError> {
        let list_blockchains_params = ListBlockchainsParams::builder()
            .test(matches!(self.chain, Chain::ArbitrumSepolia | Chain::BaseSepolia))
            .deprecated(false)
            .build();

        let blockchains = self
            .fireblocks_client
            .sdk
            .apis()
            .blockchains_assets_beta_api()
            .list_blockchains(list_blockchains_params)
            .await?;

        blockchains
            .data
            .into_iter()
            .find(|b| b.onchain.chain_id == Some(self.chain_id.to_string()))
            .map(|b| b.id)
            .ok_or(FundsManagerError::fireblocks(ERR_ARB_CHAIN_NOT_FOUND))
    }

    /// Poll a fireblocks transaction for completion
    pub(crate) async fn poll_fireblocks_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<TransactionResponse, FundsManagerError> {
        let timeout = Duration::from_secs(60);
        let interval = Duration::from_secs(1);
        self.fireblocks_client
            .sdk
            .poll_transaction(transaction_id, timeout, interval, |tx| {
                debug!("tx {}: {:?}", transaction_id, tx.status);
            })
            .await
            .map_err(FundsManagerError::fireblocks)
    }

    /// Poll Fireblocks for the indexing of an externally-initiated transaction
    /// w/ the given hash
    pub(crate) async fn poll_fireblocks_external_transaction(
        &self,
        tx_hash: TxHash,
    ) -> Result<TransactionResponse, FundsManagerError> {
        let timeout_instant = Instant::now() + Duration::from_secs(60);
        let interval = Duration::from_secs(1);

        let tx_hash_str = format!("{:#x}", tx_hash);

        let params = GetTransactionsParams::builder().tx_hash(tx_hash_str).build();

        while Instant::now() < timeout_instant {
            let txs = self
                .fireblocks_client
                .sdk
                .apis()
                .transactions_api()
                .get_transactions(params.clone())
                .await?;

            if let Some(tx) = txs.first() {
                return Ok(tx.clone());
            }

            tokio::time::sleep(interval).await;
        }

        Err(FundsManagerError::fireblocks(format!(
            "Timed out waiting for Fireblocks to index transaction {tx_hash}"
        )))
    }

    // --- JSON RPC --- //

    /// Get an instance of a signer with the http provider attached
    fn get_signing_provider(&self, wallet: PrivateKeySigner) -> DynProvider {
        build_provider(&self.rpc_url, Some(wallet))
    }

    /// Get a basic provider for the configured RPC URL, i.e. one that is unable
    /// to sign transactions
    fn get_basic_provider(&self) -> DynProvider {
        build_provider(&self.rpc_url, None /* wallet */)
    }

    /// Get the native token balance of an address
    pub(crate) async fn get_ether_balance(&self, address: &str) -> Result<f64, FundsManagerError> {
        let address = Address::from_str(address).map_err(FundsManagerError::parse)?;
        let balance = self
            .get_basic_provider()
            .get_balance(address)
            .await
            .map_err(FundsManagerError::on_chain)?;

        // Convert U256 to f64
        let balance_str = format_units(balance, "ether").map_err(FundsManagerError::parse)?;
        balance_str.parse::<f64>().map_err(FundsManagerError::parse)
    }

    /// Transfer ether from the given wallet
    pub(crate) async fn transfer_ether(
        &self,
        to: &str,
        amount: f64,
        wallet: PrivateKeySigner,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        let client = self.get_signing_provider(wallet);

        let to = Address::from_str(to).map_err(FundsManagerError::parse)?;
        let amount_units =
            parse_units(&amount.to_string(), "ether").map_err(FundsManagerError::parse)?.into();

        info!("Transferring {amount} ETH to {to:#x}");
        let tx = TransactionRequest::default().with_to(to).with_value(amount_units);
        send_tx_with_retry(tx, &client, ONE_CONFIRMATION).await
    }

    /// Get the erc20 balance of an address
    pub(crate) async fn get_erc20_balance(
        &self,
        token_address: &str,
        address: &str,
    ) -> Result<f64, FundsManagerError> {
        get_erc20_balance(token_address, address, self.get_basic_provider()).await
    }

    /// Perform an erc20 transfer
    pub(crate) async fn erc20_transfer(
        &self,
        mint: &str,
        to_address: &str,
        amount: f64,
        wallet: PrivateKeySigner,
    ) -> Result<TransactionReceipt, FundsManagerError> {
        // Setup the provider
        let client = self.get_signing_provider(wallet);
        let token_address = Address::from_str(mint).map_err(FundsManagerError::parse)?;
        let token = IERC20::new(token_address, client.clone());

        // Convert the amount using the token's decimals
        let decimals = token.decimals().call().await.map_err(FundsManagerError::on_chain)?;
        let amount =
            parse_units(&amount.to_string(), decimals).map_err(FundsManagerError::parse)?.into();

        // Transfer the tokens
        let to_address = Address::from_str(to_address).map_err(FundsManagerError::parse)?;
        let tx = token.transfer(to_address, amount);
        send_tx_with_retry(tx.into_transaction_request(), &client, FB_CONTRACT_CONFIRMATIONS).await
    }
}
