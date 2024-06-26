//! Fee redemption logic

use std::collections::HashMap;
use std::str::FromStr;

use aws_sdk_secretsmanager::Client as SecretsManagerClient;
use ethers::core::rand::thread_rng;
use ethers::signers::LocalWallet;
use ethers::utils::hex;
use renegade_common::types::wallet::derivation::{
    derive_blinder_seed, derive_share_seed, derive_wallet_id, derive_wallet_keychain,
};
use renegade_common::types::wallet::{Wallet, WalletIdentifier};
use renegade_util::raw_err_str;
use tracing::{info, warn};

use crate::db::models::WalletMetadata;
use crate::Indexer;

/// The maximum number of fees to redeem in a given run of the indexer
pub(crate) const MAX_FEES_REDEEMED: usize = 20;

impl Indexer {
    /// Redeem the most valuable open fees
    pub async fn redeem_fees(&mut self) -> Result<(), String> {
        info!("redeeming fees...");

        // Get all mints that have unredeemed fees
        let mints = self.get_unredeemed_fee_mints()?;

        // Get the prices of each redeemable mint, we want to redeem the most profitable fees first
        let mut prices = HashMap::new();
        for mint in mints.into_iter() {
            let maybe_price = self.relayer_client.get_binance_price(&mint).await?;
            if let Some(price) = maybe_price {
                prices.insert(mint, price);
            } else {
                warn!("{}: no price", mint);
            }
        }

        // Get the most valuable fees and redeem them
        let most_valuable_fees = self.get_most_valuable_fees(prices)?;

        // TODO: Filter by those fees whose present value exceeds the expected gas costs to redeem
        for fee in most_valuable_fees.into_iter() {
            let wallet = self.get_or_create_wallet(&fee.mint).await?;
            info!("redeeming into {}", wallet.id);
        }

        Ok(())
    }

    /// Find or create a wallet to store balances of a given mint
    async fn get_or_create_wallet(&mut self, mint: &str) -> Result<WalletMetadata, String> {
        let maybe_wallet = self.get_wallet_for_mint(mint)?;
        let maybe_wallet =
            maybe_wallet.or_else(|| self.find_wallet_with_empty_balance().ok().flatten());

        match maybe_wallet {
            Some(wallet) => Ok(wallet),
            None => {
                info!("creating new wallet for {mint}");
                self.create_new_wallet().await
            }
        }
    }

    /// Create a new wallet for managing a given mint
    ///
    /// Return the new wallet's metadata
    async fn create_new_wallet(&mut self) -> Result<WalletMetadata, String> {
        // 1. Create the new wallet on-chain
        let (wallet_id, root_key) = self.create_renegade_wallet().await?;

        // 2. Create a secrets manager entry for the new wallet
        let secret_name = self
            .create_secrets_manager_entry(wallet_id, root_key)
            .await?;

        // 3. Add an entry in the wallets table for the newly created wallet
        let entry = WalletMetadata::empty(wallet_id, secret_name);
        self.insert_wallet(entry.clone())?;

        Ok(entry)
    }

    /// Create a new Renegade wallet on-chain
    async fn create_renegade_wallet(&mut self) -> Result<(WalletIdentifier, LocalWallet), String> {
        let chain_id = self
            .arbitrum_client
            .chain_id()
            .await
            .map_err(raw_err_str!("Error fetching chain ID: {}"))?;
        let root_key = LocalWallet::new(&mut thread_rng());

        let wallet_id = derive_wallet_id(&root_key)?;
        let blinder_seed = derive_blinder_seed(&root_key)?;
        let share_seed = derive_share_seed(&root_key)?;
        let key_chain = derive_wallet_keychain(&root_key, chain_id)?;

        let wallet = Wallet::new_empty_wallet(wallet_id, blinder_seed, share_seed, key_chain);
        self.relayer_client.create_new_wallet(wallet).await?;
        info!("created new wallet for fee redemption");

        Ok((wallet_id, root_key))
    }

    /// Add a Renegade wallet to the secrets manager entry so that it may be recovered later
    ///
    /// Returns the name of the secret
    async fn create_secrets_manager_entry(
        &mut self,
        id: WalletIdentifier,
        wallet: LocalWallet,
    ) -> Result<String, String> {
        let client = SecretsManagerClient::new(&self.aws_config);
        let secret_name = format!("redemption-wallet-{}-{id}", self.env);
        let secret_val = hex::encode(wallet.signer().to_bytes());

        // Check that the `LocalWallet` recovers the same
        debug_assert_eq!(LocalWallet::from_str(&secret_val).unwrap(), wallet);

        // Store the secret in AWS
        client
            .create_secret()
            .name(secret_name.clone())
            .secret_string(secret_val)
            .description("Wallet used for fee redemption")
            .send()
            .await
            .map_err(raw_err_str!("Error creating secret: {}"))?;

        Ok(secret_name)
    }
}
