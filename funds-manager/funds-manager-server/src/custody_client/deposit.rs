//! Deposit funds into the custody backend

use crate::error::FundsManagerError;

use super::{CustodyClient, DepositWithdrawSource};

impl CustodyClient {
    /// Get the deposit address for the given mint
    pub(crate) async fn get_deposit_address(
        &self,
        mint: &str,
        source: DepositWithdrawSource,
    ) -> Result<String, FundsManagerError> {
        // Find a vault account for the asset
        let symbol = self.get_erc20_token_symbol(mint).await?;
        let deposit_vault = self.get_vault_account(&source).await?.ok_or_else(|| {
            FundsManagerError::fireblocks(format!(
                "no vault for deposit source: {}",
                source.get_vault_name()
            ))
        })?;

        // TODO: Create an account asset if one doesn't exist
        let asset = self.get_wallet_for_ticker(&deposit_vault, &symbol).ok_or_else(|| {
            FundsManagerError::fireblocks(format!(
                "no wallet for deposit source: {}",
                source.get_vault_name()
            ))
        })?;

        // Fetch the wallet addresses for the asset
        let client = self.get_fireblocks_client()?;
        let (addresses, _rid) = client.addresses(deposit_vault.id, &asset.id).await?;
        let addr = addresses.first().ok_or_else(|| {
            FundsManagerError::fireblocks(format!("no addresses for asset: {}", asset.id))
        })?;

        Ok(addr.address.clone())
    }
}
