//! Deposit funds into the custody backend

use fireblocks_sdk::{
    types::{Account as FireblocksAccount, AccountAsset},
    PagingVaultRequestBuilder,
};
use renegade_util::err_str;

use crate::error::FundsManagerError;

use super::{CustodyClient, DepositSource};

impl CustodyClient {
    /// Get the deposit address for the given mint
    pub(crate) async fn get_deposit_address(
        &self,
        mint: &str,
        source: DepositSource,
    ) -> Result<String, FundsManagerError> {
        // Find a vault account for the asset
        let symbol = self.get_erc20_token_symbol(mint).await?;
        let deposit_vault =
            self.get_vault_account(&source).await?.ok_or(FundsManagerError::fireblocks(
                format!("no vault for deposit source: {}", source.get_vault_name()),
            ))?;

        // TODO: Create an account asset if one doesn't exist
        let asset = self.get_wallet_for_ticker(&deposit_vault, &symbol).ok_or(
            FundsManagerError::fireblocks(format!(
                "no wallet for deposit source: {}",
                source.get_vault_name()
            )),
        )?;

        // Fetch the wallet addresses for the asset
        let client = self.get_fireblocks_client()?;
        let (addresses, _rid) = client.addresses(deposit_vault.id, &asset.id).await?;
        let addr = addresses.first().ok_or(FundsManagerError::fireblocks(format!(
            "no addresses for asset: {}",
            asset.id
        )))?;

        Ok(addr.address.clone())
    }

    /// Get the vault account for a given asset and source
    async fn get_vault_account(
        &self,
        source: &DepositSource,
    ) -> Result<Option<FireblocksAccount>, FundsManagerError> {
        let client = self.get_fireblocks_client()?;
        let req = PagingVaultRequestBuilder::new()
            .limit(100)
            .build()
            .map_err(err_str!(FundsManagerError::Fireblocks))?;

        let (vaults, _rid) = client.vaults(req).await?;
        for vault in vaults.accounts.into_iter() {
            if vault.name == source.get_vault_name() {
                return Ok(Some(vault));
            }
        }

        Ok(None)
    }

    /// Find the wallet in a vault account for a given symbol
    fn get_wallet_for_ticker(
        &self,
        vault: &FireblocksAccount,
        symbol: &str,
    ) -> Option<AccountAsset> {
        for acct in vault.assets.iter() {
            if acct.id.starts_with(symbol) {
                return Some(acct.clone());
            }
        }

        None
    }
}
