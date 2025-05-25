//! Handlers for managing Fireblocks vaults

use fireblocks_sdk::{
    apis::{
        blockchains_assets_beta_api::GetAssetByIdParams, vaults_api::GetPagedVaultAccountsParams,
        Api,
    },
    models::VaultAccount,
};
use funds_manager_api::hot_wallets::TokenBalance;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use tracing::info;

use crate::error::FundsManagerError;

use super::CustodyClient;

impl CustodyClient {
    /// Get the vault account for a given asset and source
    pub(crate) async fn get_vault_account(
        &self,
        name: &str,
    ) -> Result<Option<VaultAccount>, FundsManagerError> {
        let params = GetPagedVaultAccountsParams::builder()
            .name_prefix(name.to_string())
            .limit(100.0)
            .build();

        let vaults_resp =
            self.fireblocks_client.sdk.vaults_api().get_paged_vault_accounts(params).await?;

        for vault in vaults_resp.accounts.into_iter() {
            if vault.name == name {
                return Ok(Some(vault));
            }
        }

        Ok(None)
    }

    /// Get the non-zero balances of a vault
    pub(crate) async fn get_vault_balances(
        &self,
        vault_name: &str,
    ) -> Result<Vec<TokenBalance>, FundsManagerError> {
        let vault = self
            .get_vault_account(vault_name)
            .await?
            .ok_or(FundsManagerError::fireblocks(format!("vault {vault_name} not found")))?;

        let mut balances = Vec::new();
        for asset in vault.assets {
            let total_f64: f64 = asset.total.parse().map_err(FundsManagerError::parse)?;
            if total_f64 == 0.0 {
                continue;
            }

            let params = GetAssetByIdParams::builder().id(asset.id.clone()).build();
            let asset_resp = self
                .fireblocks_client
                .sdk
                .apis()
                .blockchains_assets_beta_api()
                .get_asset_by_id(params)
                .await?;

            let asset_onchain_data = asset_resp.onchain.ok_or(FundsManagerError::fireblocks(
                format!("asset {} has no onchain data", &asset.id),
            ))?;

            info!("asset id: {}, native asset id: {}", &asset.id, self.get_native_eth_asset_id()?);
            let mint = if asset.id == self.get_native_eth_asset_id()? {
                NATIVE_ASSET_ADDRESS.to_string()
            } else {
                asset_onchain_data.address.ok_or(FundsManagerError::fireblocks(format!(
                    "asset {} has no address",
                    &asset.id
                )))?
            };

            let amount_f64 = total_f64.powf(asset_onchain_data.decimals as f64);
            let amount: u128 = amount_f64.floor() as u128;

            balances.push(TokenBalance { mint, amount });
        }

        Ok(balances)
    }
}
