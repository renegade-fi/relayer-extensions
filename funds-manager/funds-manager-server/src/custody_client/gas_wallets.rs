//! Handlers for gas wallet operations

use std::str::FromStr;

use alloy::{hex::ToHexExt, signers::local::PrivateKeySigner};

use crate::log_task;
use crate::logger::{Outcome, Task};
use crate::{
    custody_client::DepositWithdrawSource,
    db::models::{GasWallet, GasWalletStatus},
    error::FundsManagerError,
    helpers::{create_secrets_manager_entry_with_description, get_secret},
};

use super::CustodyClient;

/// The threshold beneath which we skip refilling gas for a wallet
///
/// I.e. if the wallet's balance is within this percentage of the desired fill,
/// we skip refilling
pub const DEFAULT_GAS_REFILL_TOLERANCE: f64 = 0.1; // 10%
/// The amount to top up a newly registered gas wallet
pub const DEFAULT_TOP_UP_AMOUNT: f64 = 0.01; // ETH
/// The minimum age a gas wallet must reach before a report cycle may begin
/// transitioning it toward inactive.
///
/// This protects a slow-booting relayer worker: a wallet that was just
/// registered but whose peer has not yet appeared in the active-peers list is
/// not reclaimed out from under it within one report cycle. This is what makes
/// a short (e.g. 2-minute) report cadence safe.
pub const GAS_WALLET_RECLAIM_GRACE: std::time::Duration = std::time::Duration::from_secs(180); // 3 min

impl CustodyClient {
    // ------------
    // | Handlers |
    // ------------

    /// Refill gas for all gas wallets
    pub(crate) async fn refill_gas_wallets(&self, fill_to: f64) -> Result<(), FundsManagerError> {
        log_task!(
            Task::GasWallet,
            Outcome::Started,
            chain = %self.chain,
            fill_to = fill_to,
            "refilling {} gas wallets to {fill_to} ETH",
            self.chain
        );
        // Fetch all gas wallets
        let gas_wallets = self.get_all_gas_wallets().await?;
        // Refill the gas wallets
        self.refill_gas_for_wallets(gas_wallets, fill_to).await
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
        log_task!(
            Task::GasWallet,
            Outcome::Ok,
            subject = %address,
            "created gas wallet with address: {}",
            address
        );

        Ok(address)
    }

    /// Register a gas wallet for a peer
    ///
    /// Returns the private key the client should use for gas.
    ///
    /// Registration is idempotent per peer-id: if the peer already owns an
    /// active gas wallet we return that wallet's existing key rather than
    /// allocating a fresh inactive wallet. Returning the existing key is safe
    /// because it is the same key the peer was already issued. This prevents
    /// every relayer reboot (new peer-id consumes a wallet) from draining the
    /// inactive pool, which previously exhausted the pool and caused the
    /// `find_inactive_gas_wallet` "Record not found" 500 crash-loop.
    pub(crate) async fn register_gas_wallet(
        &self,
        peer_id: &str,
    ) -> Result<String, FundsManagerError> {
        // If this peer already has an active wallet, return its key (idempotent)
        let gas_wallet = match self.find_active_gas_wallet_for_peer(peer_id).await? {
            Some(existing) => existing,
            None => {
                // Otherwise allocate a fresh inactive wallet for the peer
                let wallet = self.find_inactive_gas_wallet().await?;
                self.mark_gas_wallet_active(&wallet.address, peer_id).await?;
                wallet
            },
        };

        let secret_name = Self::gas_wallet_secret_name(&gas_wallet.address);
        let secret_value = get_secret(&secret_name, &self.aws_config).await?;

        // Top up wallets and return the key
        self.refill_gas_wallets(self.gas_top_up_amount).await?;
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
                // Grace period: do not begin reclaiming a wallet that was registered
                // within the last `GAS_WALLET_RECLAIM_GRACE`. This protects a
                // slow-booting worker whose peer has not yet reported. Only the
                // Active->Pending step is gated; once a wallet is already Pending it
                // is older than the grace and may proceed toward Inactive.
                if state == GasWalletStatus::Active
                    && wallet
                        .created_at
                        .elapsed()
                        .map(|age| age < GAS_WALLET_RECLAIM_GRACE)
                        .unwrap_or(false)
                {
                    continue;
                }

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
        format!("gas-wallet-{}", address.to_lowercase())
    }

    /// Refill gas for a set of wallets
    async fn refill_gas_for_wallets(
        &self,
        wallets: Vec<GasWallet>,
        fill_to: f64,
    ) -> Result<(), FundsManagerError> {
        // Get the gas hot wallet's private key
        let source = DepositWithdrawSource::Gas.vault_name(self.chain);
        let gas_wallet = self.get_hot_wallet_by_vault(&source).await?;

        // Check that the gas wallet has enough ETH to cover the refill
        let my_balance = self.get_ether_balance(&gas_wallet.address).await?;
        let mut total_amount = 0.;
        for wallet in wallets.iter() {
            let bal = self.get_ether_balance(&wallet.address).await?;
            let needs = f64::max(fill_to - bal, 0.);
            total_amount += needs;
        }

        // If the gas wallet has insufficient funds, top up each wallet as much as
        // possible
        let (target, amount_desc) = if my_balance < total_amount {
            let t = my_balance / wallets.len() as f64;
            (t, format!("(hot wallet balance / {} wallets = {})", wallets.len(), t))
        } else {
            (fill_to, format!("fill_to amount {fill_to}"))
        };

        for wallet in wallets.iter() {
            self.top_up_gas(&wallet.address, "ETH", target, &amount_desc).await?;
        }
        Ok(())
    }

    /// Refill the gas wallet up to a given amount using default tolerance
    pub(crate) async fn top_up_gas(
        &self,
        addr: &str,
        symbol: &str,
        amount: f64,
        amount_desc: &str,
    ) -> Result<(), FundsManagerError> {
        self.top_up_gas_with_tolerance(addr, symbol, amount, self.gas_refill_tolerance, amount_desc)
            .await
    }

    /// Refill gas for a wallet up to a given amount
    ///
    /// Allows for a tolerance in refill amount, i.e. if the wallet's current
    /// balance is within the tolerance of the desired fill, we skip the refill
    pub(crate) async fn top_up_gas_with_tolerance(
        &self,
        addr: &str,
        symbol: &str,
        amount: f64,
        tolerance: f64,
        amount_desc: &str,
    ) -> Result<(), FundsManagerError> {
        let bal = self.get_ether_balance(addr).await?;
        if bal > amount * tolerance {
            log_task!(
                Task::GasWallet,
                Outcome::Skipped,
                subject = %addr,
                symbol = %symbol,
                balance = bal,
                target = amount,
                amount_desc = %amount_desc,
                tolerance = tolerance,
                "skipping gas refill for 0x{addr} ({symbol}) because balance is within tolerance [{tolerance} of {amount_desc}] (has {bal}, above {})",
                amount * tolerance,
            );
            return Ok(());
        }

        // Get the gas hot wallet's private key
        let source = DepositWithdrawSource::Gas.vault_name(self.chain);
        let gas_wallet = self.get_hot_wallet_by_vault(&source).await?;
        let signer = self.get_hot_wallet_private_key(&gas_wallet.address).await?;

        // Refill the balance
        let needs = amount - bal;
        self.transfer_ether(addr, needs, signer).await.map(|_| ())
    }
}
