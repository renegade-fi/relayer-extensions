//! Handlers for gas wallet operations

use ethers::{
    signers::{LocalWallet, Signer},
    utils::hex::ToHexExt,
};
use rand::thread_rng;
use tracing::info;

use crate::{
    error::FundsManagerError,
    helpers::{create_secrets_manager_entry_with_description, get_secret},
};

use super::CustodyClient;

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Create a new gas wallet
    pub(crate) async fn create_gas_wallet(&self) -> Result<String, FundsManagerError> {
        // Sample a new ethereum keypair
        let keypair = LocalWallet::new(&mut thread_rng());
        let address = keypair.address().encode_hex();

        // Add the gas wallet to the database
        self.add_gas_wallet(&address).await?;

        // Store the private key in secrets manager
        let secret_name = Self::gas_wallet_secret_name(&address);
        let private_key = keypair.signer().to_bytes();
        let secret_value = hex::encode(private_key);
        let description = "Gas wallet private key for use by Renegade relayers";
        create_secrets_manager_entry_with_description(
            &secret_name,
            &secret_value,
            &self.aws_config,
            description,
        )
        .await?;
        info!("Created gas wallet with address: {}", address);

        Ok(address)
    }

    /// Register a gas wallet for a peer
    ///
    /// Returns the private key the client should use for gas
    pub(crate) async fn register_gas_wallet(
        &self,
        peer_id: &str,
    ) -> Result<String, FundsManagerError> {
        let gas_wallet = self.find_inactive_gas_wallet().await?;
        let secret_name = Self::gas_wallet_secret_name(&gas_wallet.address);
        let secret_value = get_secret(&secret_name, &self.aws_config).await?;

        // Update the gas wallet to be active and return the keypair
        self.mark_gas_wallet_active(&gas_wallet.address, peer_id).await?;
        Ok(secret_value)
    }

    // -----------
    // | Helpers |
    // -----------

    /// Get the secret name for a gas wallet's private key
    fn gas_wallet_secret_name(address: &str) -> String {
        format!("gas-wallet-{}", address)
    }
}
