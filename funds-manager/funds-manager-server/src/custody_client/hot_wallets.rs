//! Handlers for managing hot wallets
//!
//! We store funds in hot wallets to prevent excessive in/out-flow from
//! Fireblocks

use ethers::{
    signers::{LocalWallet, Signer},
    utils::hex::ToHexExt,
};
use rand::thread_rng;
use tracing::info;

use super::CustodyClient;
use crate::{error::FundsManagerError, helpers::create_secrets_manager_entry_with_description};

impl CustodyClient {
    /// Create a new hot wallet
    ///
    /// Returns the Arbitrum address of the hot wallet
    pub async fn create_hot_wallet(&self, vault: String) -> Result<String, FundsManagerError> {
        // Generate a new Ethereum keypair
        let wallet = LocalWallet::new(&mut thread_rng());
        let address = wallet.address().encode_hex();
        let private_key = wallet.signer().to_bytes();

        // Store the private key in Secrets Manager
        let secret_name = format!("hot-wallet-{}", address);
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
}
