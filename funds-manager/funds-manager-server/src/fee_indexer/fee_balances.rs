//! Fetch the balances of redeemed fees

use crate::db::models::WalletMetadata;
use crate::error::FundsManagerError;
use renegade_api::types::ApiWallet;
use renegade_common::types::wallet::derivation::derive_wallet_keychain;

use super::Indexer;

impl Indexer {
    /// Fetch fee balances for wallets managed by the funds manager
    pub async fn fetch_fee_wallets(&mut self) -> Result<Vec<ApiWallet>, FundsManagerError> {
        // Query the wallets and fetch from the relayer
        let wallet_metadata = self.get_all_wallets().await?;
        let mut wallets = Vec::with_capacity(wallet_metadata.len());
        for meta in wallet_metadata.into_iter() {
            let wallet = self.fetch_wallet(meta).await?;
            wallets.push(wallet);
        }

        Ok(wallets)
    }

    /// Fetch a wallet given its metadata
    ///
    /// This is done by:
    ///     1. Fetch the wallet's key from secrets manager
    ///     2. Use the key to fetch the wallet from the relayer
    async fn fetch_wallet(
        &mut self,
        wallet_metadata: WalletMetadata,
    ) -> Result<ApiWallet, FundsManagerError> {
        // Get the wallet's private key from secrets manager
        let eth_key = self.get_wallet_private_key(&wallet_metadata).await?;

        // Derive the wallet keychain
        let wallet_keychain =
            derive_wallet_keychain(&eth_key, self.chain_id).map_err(FundsManagerError::custom)?;
        let root_key = wallet_keychain.secret_keys.sk_root.clone().expect("root key not present");

        // Fetch the wallet from the relayer
        let wallet = self.relayer_client.get_wallet(wallet_metadata.id, &root_key).await?;
        Ok(wallet.wallet)
    }
}
