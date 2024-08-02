//! Handlers for gas wallet operations

use ethers::{
    signers::{LocalWallet, Signer},
    utils::hex::ToHexExt,
};
use rand::thread_rng;
use tracing::info;

use crate::{error::FundsManagerError, helpers::create_secrets_manager_entry_with_description};

use super::CustodyClient;

impl CustodyClient {
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

    /// Get the secret name for a gas wallet's private key
    fn gas_wallet_secret_name(address: &str) -> String {
        format!("gas-wallet-{}", address)
    }
}
