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
        let vault_name = source.vault_name();
        self.get_deposit_address_by_vault_name(mint, vault_name).await
    }

    /// Get the deposit address given a vault name
    pub(crate) async fn get_deposit_address_by_vault_name(
        &self,
        mint: &str,
        vault_name: &str,
    ) -> Result<String, FundsManagerError> {
        // Find a vault account for the asset
        let symbol = self.get_erc20_token_symbol(mint).await?;
        let deposit_vault = self.get_vault_account(vault_name).await?.ok_or_else(|| {
            FundsManagerError::fireblocks(format!("no vault for deposit source: {vault_name}"))
        })?;

        // TODO: Create an account asset if one doesn't exist
        let asset = self.get_wallet_for_ticker(&deposit_vault, &symbol).ok_or_else(|| {
            FundsManagerError::fireblocks(format!("no wallet for deposit source: {vault_name}"))
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
