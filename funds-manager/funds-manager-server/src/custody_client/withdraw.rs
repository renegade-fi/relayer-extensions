use crate::error::FundsManagerError;
use bigdecimal::{BigDecimal, FromPrimitive};
use fireblocks_sdk::types::TransactionStatus;

use super::{CustodyClient, DepositWithdrawSource};

impl CustodyClient {
    /// Withdraw gas from custody
    pub(crate) async fn withdraw_with_token_addr(
        &self,
        source: DepositWithdrawSource,
        destination_address: &str,
        token_address: &str,
        amount: f64,
    ) -> Result<(), FundsManagerError> {
        let symbol = self.get_erc20_token_symbol(token_address).await?;
        self.withdraw(source, destination_address, &symbol, amount).await
    }

    /// Withdraw funds from custody
    pub(crate) async fn withdraw(
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
}
