//! Fee redemption logic

use std::collections::HashMap;
use std::str::FromStr;

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::TxHash;
use aws_sdk_secretsmanager::Client as SecretsManagerClient;
use renegade_api::http::wallet::RedeemNoteRequest;
use renegade_circuit_types::note::Note;
use renegade_common::types::wallet::derivation::{
    derive_blinder_seed, derive_share_seed, derive_wallet_id, derive_wallet_keychain,
};
use renegade_common::types::wallet::{Wallet, WalletIdentifier};
use renegade_util::err_str;
use tracing::{info, warn};

use crate::db::models::RenegadeWalletMetadata;
use crate::error::FundsManagerError;
use crate::helpers::{create_secrets_manager_entry_with_description, get_secret_prefix};
use crate::Indexer;

/// The maximum number of fees to redeem in a given run of the indexer
pub(crate) const MAX_FEES_REDEEMED: usize = 100;

impl Indexer {
    /// Redeem the most valuable open fees
    pub async fn redeem_fees(&self) -> Result<(), FundsManagerError> {
        info!("redeeming fees...");

        // Get all mints that have unredeemed fees
        let mints = self.get_unredeemed_fee_mints().await?;

        // Get the prices of each redeemable mint, we want to redeem the most profitable
        // fees first
        let mut prices = HashMap::new();
        for mint in mints.into_iter() {
            let maybe_price = self.price_reporter.get_price(&mint, self.chain).await;
            match maybe_price {
                Ok(price) => {
                    prices.insert(mint, price);
                },
                Err(e) => {
                    warn!("{}: error getting price: {e}", mint);
                },
            }
        }

        // Get the most valuable fees and redeem them
        let most_valuable_fees = self.get_most_valuable_fees(prices).await?;

        // TODO: Filter by those fees whose present value exceeds the expected gas costs
        // to redeem
        for fee in most_valuable_fees.into_iter() {
            let wallet = self.get_or_create_wallet(&fee.mint).await?;
            self.redeem_note_into_wallet(fee.tx_hash.clone(), fee.receiver, wallet).await?;
        }

        Ok(())
    }

    // -------------------
    // | Wallet Creation |
    // -------------------

    /// Find or create a wallet to store balances of a given mint
    async fn get_or_create_wallet(
        &self,
        mint: &str,
    ) -> Result<RenegadeWalletMetadata, FundsManagerError> {
        // Find a wallet with an existing balance
        let maybe_wallet = self.get_wallet_for_mint(mint).await?;
        if let Some(wallet) = maybe_wallet {
            return Ok(wallet);
        }

        // Otherwise find a wallet with an empty balance slot, create a new one if no
        // such wallet exists
        let maybe_wallet = self.find_wallet_with_empty_balance().await?;
        let wallet = match maybe_wallet {
            Some(wallet) => wallet,
            None => {
                info!("creating new wallet for {mint}");
                self.create_new_wallet().await?
            },
        };

        self.add_mint_to_wallet(&wallet.id, mint).await?;
        Ok(wallet)
    }

    /// Create a new wallet for managing a given mint
    ///
    /// Return the new wallet's metadata
    async fn create_new_wallet(&self) -> Result<RenegadeWalletMetadata, FundsManagerError> {
        // 1. Create the new wallet on-chain
        let (wallet_id, root_key) = self.create_renegade_wallet().await?;

        // 2. Create a secrets manager entry for the new wallet
        let secret_name = self.store_wallet_secret(wallet_id, root_key).await?;

        // 3. Add an entry in the wallets table for the newly created wallet
        let entry = RenegadeWalletMetadata::empty(wallet_id, secret_name, self.chain);
        self.insert_wallet(entry.clone()).await?;

        Ok(entry)
    }

    /// Create a new Renegade wallet on-chain
    async fn create_renegade_wallet(
        &self,
    ) -> Result<(WalletIdentifier, PrivateKeySigner), FundsManagerError> {
        let root_key = PrivateKeySigner::random();

        let wallet_id = derive_wallet_id(&root_key).map_err(FundsManagerError::custom)?;
        let blinder_seed = derive_blinder_seed(&root_key).map_err(FundsManagerError::custom)?;
        let share_seed = derive_share_seed(&root_key).map_err(FundsManagerError::custom)?;
        let key_chain =
            derive_wallet_keychain(&root_key, self.chain_id).map_err(FundsManagerError::custom)?;

        let wallet = Wallet::new_empty_wallet(wallet_id, blinder_seed, share_seed, key_chain);
        self.relayer_client.create_new_wallet(wallet, &blinder_seed).await?;
        info!("created new wallet for fee redemption");

        Ok((wallet_id, root_key))
    }

    // ------------------
    // | Fee Redemption |
    // ------------------

    /// Redeem a note into a wallet
    pub async fn redeem_note_into_wallet(
        &self,
        tx: String,
        receiver: String,
        wallet: RenegadeWalletMetadata,
    ) -> Result<Note, FundsManagerError> {
        info!("redeeming fee into {}", wallet.id);
        // Get the wallet key for the given wallet
        let eth_key = self.get_wallet_private_key(&wallet).await?;
        let wallet_keychain = derive_wallet_keychain(&eth_key, self.chain_id).unwrap();
        let wallet_key = wallet_keychain.symmetric_key();

        // Fetch the wallet, ensuring it is looked up on the relayer
        self.relayer_client.get_wallet(wallet.id, &eth_key, wallet_keychain).await?;

        // Find the note in the tx body
        let tx_hash = TxHash::from_str(&tx).map_err(err_str!(FundsManagerError::Parse))?;
        let key = self
            .get_key_for_receiver(&receiver)
            .ok_or(FundsManagerError::custom("no key found for receiver"))?;
        let note = self.get_note_from_tx_with_key(tx_hash, key).await?;

        // Redeem the note through the relayer
        let req = RedeemNoteRequest { note: note.clone(), decryption_key: *key };
        self.relayer_client.redeem_note(wallet.id, req, &wallet_key).await?;

        // Mark the fee as redeemed
        self.maybe_mark_redeemed(&tx, &note).await?;
        Ok(note)
    }

    /// Mark a fee as redeemed if its nullifier is spent on-chain
    async fn maybe_mark_redeemed(
        &self,
        tx_hash: &str,
        note: &Note,
    ) -> Result<(), FundsManagerError> {
        let nullifier = note.nullifier();
        if !self
            .darkpool_client
            .check_nullifier_used(nullifier)
            .await
            .map_err(err_str!(FundsManagerError::on_chain))?
        {
            warn!("nullifier not seen on-chain after redemption, tx: {tx_hash}");
            return Ok(());
        }

        info!("successfully redeemed fee from tx: {}", tx_hash);
        self.mark_fee_as_redeemed(tx_hash).await
    }

    // -------------------
    // | Secrets Manager |
    // -------------------

    /// Add a Renegade wallet to the secrets manager entry so that it may be
    /// recovered later
    ///
    /// Returns the name of the secret
    async fn store_wallet_secret(
        &self,
        id: WalletIdentifier,
        wallet: PrivateKeySigner,
    ) -> Result<String, FundsManagerError> {
        let secret_name = self.get_wallet_secret_name(id)?;
        let secret_val = hex::encode(wallet.to_bytes());

        // Check that the `PrivateKeySigner` recovers the same
        debug_assert_eq!(PrivateKeySigner::from_str(&secret_val).unwrap(), wallet);
        create_secrets_manager_entry_with_description(
            &secret_name,
            &secret_val,
            &self.aws_config,
            "Renegade wallet key used for fee redemption",
        )
        .await?;
        Ok(secret_name)
    }

    /// Get the private key for a wallet specified by its metadata
    pub(crate) async fn get_wallet_private_key(
        &self,
        metadata: &RenegadeWalletMetadata,
    ) -> Result<PrivateKeySigner, FundsManagerError> {
        let client = SecretsManagerClient::new(&self.aws_config);
        let secret_name = self.get_wallet_secret_name(metadata.id)?;

        let secret = client
            .get_secret_value()
            .secret_id(secret_name)
            .send()
            .await
            .map_err(err_str!(FundsManagerError::SecretsManager))?;

        let secret_str = secret.secret_string().unwrap();
        let wallet =
            PrivateKeySigner::from_str(secret_str).map_err(err_str!(FundsManagerError::Parse))?;
        Ok(wallet)
    }

    /// Get the secret name for a wallet
    fn get_wallet_secret_name(&self, id: WalletIdentifier) -> Result<String, FundsManagerError> {
        Ok(format!("{}/redemption-wallet-{}", get_secret_prefix(self.chain)?, id))
    }
}
