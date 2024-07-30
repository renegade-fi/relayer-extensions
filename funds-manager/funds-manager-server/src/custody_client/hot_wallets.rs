//! Handlers for managing hot wallets
//!
//! We store funds in hot wallets to prevent excessive in/out-flow from
//! Fireblocks

use std::{str::FromStr, sync::Arc};

use ethers::{
    providers::{Http, Provider},
    signers::{LocalWallet, Signer},
    types::Address,
    utils::hex::ToHexExt,
};
use funds_manager_api::{TokenBalance, WalletWithBalances};
use rand::thread_rng;
use tracing::info;

use super::{CustodyClient, ERC20};
use crate::{
    custody_client::DepositWithdrawSource,
    error::FundsManagerError,
    helpers::{create_secrets_manager_entry_with_description, get_secret},
};

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Create a new hot wallet
    ///
    /// Returns the Arbitrum address of the hot wallet
    pub async fn create_hot_wallet(&self, vault: String) -> Result<String, FundsManagerError> {
        // Generate a new Ethereum keypair
        let wallet = LocalWallet::new(&mut thread_rng());
        let address = wallet.address().encode_hex();
        let private_key = wallet.signer().to_bytes();

        // Store the private key in Secrets Manager
        let secret_name = Self::hot_wallet_secret_name(&address);
        let secret_value = hex::encode(private_key);
        let description = format!("Hot wallet for vault: {vault}");
        create_secrets_manager_entry_with_description(
            &secret_name,
            &secret_value,
            &self.aws_config,
            &description,
        )
        .await?;

        // Insert the wallet metadata into the database
        self.insert_hot_wallet(&address, &vault, &secret_name).await?;
        info!("Created hot wallet with address: {} for vault: {}", address, vault);
        Ok(address)
    }

    /// Get balances for all hot wallets
    pub async fn get_hot_wallet_balances(
        &self,
        mints: &[String],
    ) -> Result<Vec<WalletWithBalances>, FundsManagerError> {
        let hot_wallets = self.get_all_hot_wallets().await?;
        let provider = Arc::new(self.get_rpc_provider()?);

        let mut hot_wallet_balances = Vec::new();
        for wallet in hot_wallets.iter().map(|w| w.address.clone()) {
            // Fetch token balances for the wallet
            let mut balances = Vec::new();
            for mint in mints.iter() {
                let balance = self.get_token_balance(&wallet, mint, provider.clone()).await?;
                balances.push(TokenBalance { mint: mint.clone(), amount: balance });
            }

            hot_wallet_balances.push(WalletWithBalances { address: wallet, balances });
        }

        Ok(hot_wallet_balances)
    }

    /// Transfer funds from a hot wallet to its backing Fireblocks vault
    pub async fn transfer_from_hot_wallet_to_vault(
        &self,
        hot_wallet_address: &str,
        mint: &str,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        // 1. Look up the wallet's information
        let hot_wallet = self.get_hot_wallet_by_address(hot_wallet_address).await?;

        // 2. Retrieve the wallet's private key from Secrets Manager
        let secret_value = get_secret(&hot_wallet.secret_id, &self.aws_config).await?;
        let wallet = LocalWallet::from_str(&secret_value).map_err(FundsManagerError::parse)?;

        // 3. Look up the vault deposit address
        let deposit_address =
            self.get_deposit_address_by_vault_name(mint, &hot_wallet.vault).await?;

        // 4. Transfer the tokens
        let receipt = self.erc20_transfer(mint, &deposit_address, amount, wallet).await?;
        info!(
            "Transferred {} of token {} from hot wallet {} to vault address {}. \n\tTransaction hash: {:#x}",
            amount, mint, hot_wallet_address, deposit_address, receipt.transaction_hash
        );

        Ok(())
    }

    pub async fn transfer_from_vault_to_hot_wallet(
        &self,
        vault: &str,
        mint: &str,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        // Fetch the wallet info, then withdraw
        let wallet = self.get_hot_wallet_by_vault(vault).await?;
        let source = DepositWithdrawSource::from_vault_name(vault)?;
        self.withdraw_with_token_addr(source, &wallet.address, mint, amount).await
    }

    // ------------
    // | Handlers |
    // ------------

    /// The secret name for a hot wallet
    fn hot_wallet_secret_name(address: &str) -> String {
        format!("hot-wallet-{address}")
    }

    /// Fetch the token balance at the given address for a wallet
    async fn get_token_balance(
        &self,
        wallet_address: &str,
        token_address: &str,
        provider: Arc<Provider<Http>>,
    ) -> Result<u128, FundsManagerError> {
        let wallet_address: Address = wallet_address.parse().map_err(|_| {
            FundsManagerError::parse(format!("Invalid wallet address: {wallet_address}"))
        })?;
        let token_address: Address = token_address.parse().map_err(|_| {
            FundsManagerError::parse(format!("Invalid token address: {token_address}"))
        })?;

        let token = ERC20::new(token_address, provider);
        token
            .balance_of(wallet_address)
            .call()
            .await
            .map(|balance| balance.as_u128())
            .map_err(FundsManagerError::arbitrum)
    }
}
