//! Handlers for managing Fireblocks vaults

use alloy_primitives::Address;
use fireblocks_sdk::{
    apis::{
        blockchains_assets_beta_api::GetAssetByIdParams, vaults_api::GetPagedVaultAccountsParams,
        Api,
    },
    models::{AssetOnchainBeta, VaultAccount, VaultAsset},
};
use funds_manager_api::hot_wallets::TokenBalance;
use futures::future::try_join_all;

use crate::error::FundsManagerError;

use super::CustodyClient;

impl CustodyClient {
    /// Get the ID of a vault by name
    pub(crate) async fn get_vault_id(&self, name: &str) -> Result<String, FundsManagerError> {
        if let Some(vault_id) = self.fireblocks_client.read_cached_vault_id(name).await {
            return Ok(vault_id);
        }

        let vault = self
            .get_vault_account(name)
            .await?
            .ok_or(FundsManagerError::fireblocks(format!("no vault with name '{name}'")))?;

        self.fireblocks_client.cache_vault_id(name.to_string(), vault.id.clone()).await;

        Ok(vault.id)
    }

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

    /// Get the non-zero token balances of a vault
    pub(crate) async fn get_vault_token_balances(
        &self,
        vault_name: &str,
    ) -> Result<Vec<TokenBalance>, FundsManagerError> {
        let vault = self
            .get_vault_account(vault_name)
            .await?
            .ok_or(FundsManagerError::fireblocks(format!("vault {vault_name} not found")))?;

        let futures =
            vault.assets.into_iter().map(|asset| self.try_get_token_balance_for_asset(asset));

        let balances = try_join_all(futures).await?;

        Ok(balances.into_iter().flatten().collect())
    }

    /// Try to construct a `TokenBalance` for a given asset.
    ///
    /// For native assets, this will return a `TokenBalance` with a zero mint
    /// address.
    ///
    /// If the asset has a zero balance, or is an unsupported native asset,
    /// this will return `None`.
    async fn try_get_token_balance_for_asset(
        &self,
        asset: VaultAsset,
    ) -> Result<Option<TokenBalance>, FundsManagerError> {
        let total_f64: f64 = asset.total.parse().map_err(FundsManagerError::parse)?;
        if total_f64 == 0.0 {
            return Ok(None);
        }

        let asset_onchain_data = self.get_asset_onchain_data(&asset.id).await?;

        // We use the zero address to represent native assets
        let mint = if self.get_current_env_native_asset_ids()?.contains(&asset.id.as_str()) {
            format!("{:#x}", Address::ZERO)
        } else if let Some(address) = asset_onchain_data.address {
            address
        } else {
            // Skip any unsupported native assets
            return Ok(None);
        };

        let amount_f64 = total_f64 * 10_f64.powf(asset_onchain_data.decimals as f64);
        let amount: u128 = amount_f64.floor() as u128;

        Ok(Some(TokenBalance { mint, amount }))
    }

    /// Get the onchain data for an asset
    async fn get_asset_onchain_data(
        &self,
        asset_id: &str,
    ) -> Result<AssetOnchainBeta, FundsManagerError> {
        if let Some(asset_onchain_data) =
            self.fireblocks_client.read_cached_asset_onchain_data(asset_id).await
        {
            return Ok(asset_onchain_data);
        }

        let params = GetAssetByIdParams::builder().id(asset_id.to_string()).build();
        let asset_resp = self
            .fireblocks_client
            .sdk
            .apis()
            .blockchains_assets_beta_api()
            .get_asset_by_id(params)
            .await?;

        let asset_onchain_data = asset_resp.onchain.ok_or(FundsManagerError::fireblocks(
            format!("asset {} has no onchain data", &asset_id),
        ))?;

        self.fireblocks_client
            .cache_asset_onchain_data(asset_id.to_string(), asset_onchain_data.clone())
            .await;

        Ok(asset_onchain_data)
    }
}
