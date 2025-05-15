//! Handlers for gas wallet operations

use std::str::FromStr;

use alloy::{hex::ToHexExt, signers::local::PrivateKeySigner};
use tracing::info;

use crate::{
    custody_client::DepositWithdrawSource,
    db::models::GasWalletStatus,
    error::FundsManagerError,
    helpers::{create_secrets_manager_entry_with_description, get_secret},
};

use super::CustodyClient;

/// The threshold beneath which we skip refilling gas for a wallet
///
/// I.e. if the wallet's balance is within this amount of the desired fill, we
/// skip refilling
pub const GAS_REFILL_TOLERANCE: f64 = 0.001; // ETH
/// The amount to top up a newly registered gas wallet
pub const DEFAULT_TOP_UP_AMOUNT: f64 = 0.01; // ETH

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Refill gas for all gas wallets
    pub(crate) async fn refill_gas_wallets(&self, fill_to: f64) -> Result<(), FundsManagerError> {
        // Fetch all gas wallets
        let gas_wallets = self.get_all_gas_wallets().await?;

        // Filter out those that don't need refilling
        let mut wallets_to_fill: Vec<(String, f64)> = Vec::new(); // (address, fill amount)
        for wallet in gas_wallets.into_iter() {
            let bal = self.get_ether_balance(&wallet.address).await?;
            if bal + GAS_REFILL_TOLERANCE < fill_to {
                wallets_to_fill.push((wallet.address, fill_to - bal));
            }
        }

        if wallets_to_fill.is_empty() {
            return Ok(());
        }

        // Refill the gas wallets
        self.refill_gas_for_wallets(wallets_to_fill).await
    }

    /// Create a new gas wallet
    pub(crate) async fn create_gas_wallet(&self) -> Result<String, FundsManagerError> {
        // Sample a new ethereum keypair
        let keypair = PrivateKeySigner::random();
        let address = keypair.address().encode_hex();

        // Add the gas wallet to the database
        self.add_gas_wallet(&address).await?;

        // Store the private key in secrets manager
        let secret_name = Self::gas_wallet_secret_name(&address);
        let private_key = keypair.credential().to_bytes();
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

        // Update the gas wallet to be active, top up wallets, and return the key
        self.mark_gas_wallet_active(&gas_wallet.address, peer_id).await?;
        self.refill_gas_wallets(DEFAULT_TOP_UP_AMOUNT).await?;
        Ok(secret_value)
    }

    /// Record the set of active peers, marking their gas wallets as active and
    /// transitioning the rest to inactive or pending if necessary
    pub(crate) async fn record_active_gas_wallet(
        &self,
        active_peers: Vec<String>,
    ) -> Result<(), FundsManagerError> {
        // Fetch all gas wallets
        let all_wallets = self.get_all_gas_wallets().await?;

        // For those gas wallets whose peer is not in the active peers list, mark them
        // as inactive
        for wallet in all_wallets {
            let state =
                GasWalletStatus::from_str(&wallet.status).expect("invalid gas wallet status");
            let peer_id = match wallet.peer_id {
                Some(peer_id) => peer_id,
                None => continue,
            };

            if !active_peers.contains(&peer_id) {
                match state.transition_inactive() {
                    GasWalletStatus::Pending => {
                        self.mark_gas_wallet_pending(&wallet.address).await?;
                    },
                    GasWalletStatus::Inactive => {
                        self.mark_gas_wallet_inactive(&wallet.address).await?;
                    },
                    _ => unreachable!(),
                }
            }
        }

        Ok(())
    }

    // -----------
    // | Helpers |
    // -----------

    /// Get the secret name for a gas wallet's private key
    fn gas_wallet_secret_name(address: &str) -> String {
        format!("gas-wallet-{}", address)
    }

    /// Refill gas for a set of wallets
    async fn refill_gas_for_wallets(
        &self,
        wallets: Vec<(String, f64)>, // (address, fill amount)
    ) -> Result<(), FundsManagerError> {
        // Get the gas hot wallet's private key
        let source = DepositWithdrawSource::Gas.vault_name(self.chain);
        let gas_wallet = self.get_hot_wallet_by_vault(&source).await?;
        let signer = self.get_hot_wallet_private_key(&gas_wallet.address).await?;

        // Check that the gas wallet has enough ETH to cover the refill
        let total_amount = wallets.iter().map(|(_, amount)| *amount).sum::<f64>();
        let my_balance = self.get_ether_balance(&gas_wallet.address).await?;
        if my_balance < total_amount {
            return Err(FundsManagerError::custom(
                "gas wallet does not have enough ETH to cover the refill",
            ));
        }

        // Refill the balances
        for (address, amount) in wallets {
            self.transfer_ether(&address, amount, signer.clone()).await?;
        }

        Ok(())
    }
}
