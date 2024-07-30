use std::str::FromStr;

use crate::{error::FundsManagerError, helpers::get_secret};
use bigdecimal::{BigDecimal, FromPrimitive};
use ethers::signers::LocalWallet;
use fireblocks_sdk::types::TransactionStatus;
use tracing::info;

use super::{CustodyClient, DepositWithdrawSource};

impl CustodyClient {
    /// Withdraw from hot wallet custody with a provided token address
    pub(crate) async fn withdraw_from_hot_wallet(
        &self,
        source: DepositWithdrawSource,
        destination_address: &str,
        token_address: &str,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        // Find the wallet for the given destination and check its balance
        let wallet = self.get_hot_wallet_by_vault(source.vault_name()).await?;
        let bal = self.get_erc20_balance(token_address, &wallet.address).await?;
        if bal < amount {
            return Err(FundsManagerError::Custom("Insufficient balance".to_string()));
        }

        // Fetch the wallet private key
        let secret_name = Self::hot_wallet_secret_name(&wallet.address);
        let private_key = get_secret(&secret_name, &self.aws_config).await?;
        let wallet =
            LocalWallet::from_str(private_key.as_str()).map_err(FundsManagerError::parse)?;

        // Execute the erc20 transfer
        let tx = self.erc20_transfer(token_address, destination_address, amount, wallet).await?;
        info!(
            "Withdrew {amount} {token_address} from hot wallet to {destination_address}. Tx: {:#}",
            tx.transaction_hash
        );

        Ok(())
    }

    /// Withdraw funds from custody
    pub(crate) async fn withdraw_from_fireblocks(
        &self,
        source: DepositWithdrawSource,
        destination_address: &str,
        symbol: &str,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        let client = self.get_fireblocks_client()?;

        // Get the vault account and asset to transfer from
        let vault = self
            .get_vault_account(source.vault_name())
            .await?
            .ok_or_else(|| FundsManagerError::Custom("Vault not found".to_string()))?;

        let asset = self.get_wallet_for_ticker(&vault, symbol).ok_or_else(|| {
            FundsManagerError::Custom(format!("Asset not found for symbol: {}", symbol))
        })?;

        // Check if the available balance is sufficient
        let withdraw_amount = BigDecimal::from_f64(amount)
            .ok_or_else(|| FundsManagerError::Custom("Invalid amount".to_string()))?;
        if asset.available < withdraw_amount {
            return Err(FundsManagerError::Custom(format!(
                "Insufficient balance. Available: {}, Requested: {}",
                asset.available, withdraw_amount
            )));
        }

        // Transfer
        let vault_name = source.vault_name();
        let note = format!("Withdraw {amount} {symbol} from {vault_name} to {destination_address}");

        let (resp, _rid) = client
            .create_transaction_external(
                vault.id,
                destination_address,
                asset.id,
                withdraw_amount,
                Some(&note),
            )
            .await?;

        let tx = self.poll_fireblocks_transaction(&resp.id).await?;
        if tx.status != TransactionStatus::COMPLETED && tx.status != TransactionStatus::CONFIRMING {
            let err_msg = format!("Transaction failed: {:?}", tx.status);
            return Err(FundsManagerError::Custom(err_msg));
        }

        Ok(())
    }

    /// Withdraw gas
    pub(crate) async fn withdraw_gas(
        &self,
        amount: f64,
        to: &str,
    ) -> Result<(), FundsManagerError> {
        // Check the gas wallet's balance
        let gas_vault_name = DepositWithdrawSource::Gas.vault_name();
        let gas_wallet = self.get_hot_wallet_by_vault(gas_vault_name).await?;
        let bal = self.get_ether_balance(&gas_wallet.address).await?;
        if bal < amount {
            return Err(FundsManagerError::custom("Insufficient balance"));
        }

        // Fetch the gas wallet's private key
        let secret_name = Self::hot_wallet_secret_name(&gas_wallet.address);
        let private_key = get_secret(&secret_name, &self.aws_config).await?;
        let wallet =
            LocalWallet::from_str(private_key.as_str()).map_err(FundsManagerError::parse)?;

        // Execute the transfer
        let tx = self.transfer_ether(to, amount, wallet).await?;
        info!("Withdrew {amount} ETH from gas wallet to {to}. Tx: {:#}", tx.transaction_hash);

        Ok(())
    }
}
