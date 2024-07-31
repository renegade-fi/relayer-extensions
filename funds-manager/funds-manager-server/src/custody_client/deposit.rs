//! Deposit funds into the custody backend

use crate::error::FundsManagerError;

use super::{CustodyClient, DepositWithdrawSource};

impl CustodyClient {
    /// Get the deposit address for the given mint
    pub(crate) async fn get_deposit_address(
        &self,
        source: DepositWithdrawSource,
    ) -> Result<String, FundsManagerError> {
        let vault_name = source.vault_name();
        self.get_deposit_address_by_vault_name(vault_name).await
    }

    /// Get the deposit address given a vault name
    pub(crate) async fn get_deposit_address_by_vault_name(
        &self,
        vault_name: &str,
    ) -> Result<String, FundsManagerError> {
        self.get_hot_wallet_by_vault(vault_name).await.map(|w| w.address)
    }

    /// Get the deposit address given a vault name
    pub(crate) async fn get_fireblocks_deposit_address(
        &self,
        mint: &str,
        vault_name: &str,
    ) -> Result<String, FundsManagerError> {
        // Find a vault account and asset
        let deposit_vault = self.get_vault_account(vault_name).await?.ok_or_else(|| {
            FundsManagerError::fireblocks(format!("no vault for deposit source: {vault_name}"))
        })?;

        let asset_id = self
            .get_asset_id_for_address(mint)
            .await?
            .ok_or_else(|| FundsManagerError::fireblocks(format!("no asset for mint: {mint}")))?;

        // Fetch the wallet addresses for the asset
        let client = self.get_fireblocks_client()?;
        let (addresses, _rid) = client.addresses(deposit_vault.id, &asset_id).await?;
        let addr = addresses.first().ok_or_else(|| {
            FundsManagerError::fireblocks(format!("no addresses for asset: {}", asset_id))
        })?;

        Ok(addr.address.clone())
    }
}
