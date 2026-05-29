//! Handlers for managing Fireblocks vaults

use alloy_primitives::Address;
use fireblocks_sdk::{
    apis::{
        blockchains_assets_beta_api::GetAssetByIdParams,
        vaults_api::{GetPagedVaultAccountsParams, GetVaultAccountAssetParams},
        Api,
    },
    models::{AssetOnchainBeta, VaultAccount, VaultAsset},
};
use funds_manager_api::hot_wallets::TokenBalance;
use futures::stream::{self, StreamExt, TryStreamExt};

use crate::error::FundsManagerError;

use super::CustodyClient;

/// Max concurrent Fireblocks per-asset metadata fetches when warming the vault
/// balance cache. An unbounded fan-out (one `get_asset_onchain_data` per asset)
/// thundering-herds the rate limiter on cold cache and triggers 429s (the
/// 2026-05-29 get-vault-balances failures). On-chain asset data is cached
/// permanently, so this only throttles the cold-start fetch; warm calls are
/// served from cache.
const VAULT_BALANCE_FETCH_CONCURRENCY: usize = 4;

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

        let vaults_resp = self
            .fireblocks_client
            .rate_limited(
                |sdk| async move { sdk.vaults_api().get_paged_vault_accounts(params).await },
            )
            .await?;

        for vault in vaults_resp.accounts.into_iter() {
            if vault.name == name {
                return Ok(Some(vault));
            }
        }

        Ok(None)
    }

    /// Get the available balance of a given asset in a vault
    pub(crate) async fn get_vault_available_balance(
        &self,
        vault_id: String,
        mint: &str,
    ) -> Result<f64, FundsManagerError> {
        let asset_id = self
            .get_asset_id_for_address(mint)
            .await?
            .ok_or(FundsManagerError::fireblocks(format!("asset {mint} not found")))?;

        let params = GetVaultAccountAssetParams::builder()
            .vault_account_id(vault_id)
            .asset_id(asset_id)
            .build();

        let vault_asset = self
            .fireblocks_client
            .rate_limited(
                |sdk| async move { sdk.vaults_api().get_vault_account_asset(params).await },
            )
            .await?;

        let available: f64 = vault_asset.available.parse().map_err(FundsManagerError::parse)?;

        Ok(available)
    }

    /// Get the non-zero token balances of a vault.
    ///
    /// Results are cached for a few seconds to coalesce retry storms from
    /// the gardener-side fetch-holdings loop. The cache is a read-side
    /// optimization only; mutating paths (deposit/withdraw/transfer) do
    /// not invalidate it, so callers must tolerate up to
    /// `VAULT_BALANCES_CACHE_TTL` of staleness.
    pub(crate) async fn get_vault_token_balances(
        &self,
        vault_name: &str,
    ) -> Result<Vec<TokenBalance>, FundsManagerError> {
        if let Some(cached) = self.fireblocks_client.read_cached_vault_balances(vault_name).await {
            return Ok(cached);
        }

        let vault = self
            .get_vault_account(vault_name)
            .await?
            .ok_or(FundsManagerError::fireblocks(format!("vault {vault_name} not found")))?;

        // Fetch each asset's balance with bounded concurrency. The per-asset
        // `get_asset_onchain_data` calls are rate-limited Fireblocks requests;
        // an unbounded fan-out thundering-herds the limiter on cold cache and
        // 429s (the 2026-05-29 failures). `try_collect` keeps fail-closed
        // semantics — one genuinely-failing asset still aborts the whole call,
        // so the caller's holdings leg fails rather than silently under-counting
        // (invariant A2). On-chain data is cached permanently, so this fan-out
        // only runs on a cold cache.
        let balances: Vec<TokenBalance> = stream::iter(vault.assets)
            .map(|asset| self.try_get_token_balance_for_asset(asset))
            .buffer_unordered(VAULT_BALANCE_FETCH_CONCURRENCY)
            .try_collect::<Vec<_>>()
            .await?
            .into_iter()
            .flatten()
            .collect();

        self.fireblocks_client.cache_vault_balances(vault_name.to_string(), balances.clone()).await;

        Ok(balances)
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
        let available_f64: f64 = asset.available.parse().map_err(FundsManagerError::parse)?;
        if available_f64 == 0.0 {
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

        let amount_f64 = available_f64 * 10_f64.powf(asset_onchain_data.decimals);
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
            .rate_limited(|sdk| async move {
                sdk.apis().blockchains_assets_beta_api().get_asset_by_id(params).await
            })
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
