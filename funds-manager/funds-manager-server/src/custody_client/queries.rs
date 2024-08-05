//! Queries for managing custody data

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_util::err_str;
use uuid::Uuid;

use crate::db::models::{GasWallet, HotWallet};
use crate::db::schema::gas_wallets;
use crate::db::schema::hot_wallets;
use crate::error::FundsManagerError;
use crate::CustodyClient;

impl CustodyClient {
    // --- Gas Wallets --- //

    /// Add a new gas wallet
    pub async fn add_gas_wallet(&self, address: &str) -> Result<(), FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        let entry = GasWallet::new(address.to_string());
        diesel::insert_into(gas_wallets::table)
            .values(entry)
            .execute(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))?;

        Ok(())
    }

    /// Find an inactive gas wallet
    pub async fn find_inactive_gas_wallet(&self) -> Result<GasWallet, FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        gas_wallets::table
            .filter(gas_wallets::active.eq(false))
            .first::<GasWallet>(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))
    }

    /// Mark a gas wallet as active
    pub async fn mark_gas_wallet_active(
        &self,
        address: &str,
        peer_id: &str,
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        let updates = (gas_wallets::active.eq(true), gas_wallets::peer_id.eq(peer_id));
        diesel::update(gas_wallets::table.filter(gas_wallets::address.eq(address)))
            .set(updates)
            .execute(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))?;

        Ok(())
    }

    // --- Hot Wallets --- //

    /// Get all hot wallets
    pub async fn get_all_hot_wallets(&self) -> Result<Vec<HotWallet>, FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        let wallets = hot_wallets::table
            .load::<HotWallet>(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))?;

        Ok(wallets)
    }

    /// Insert a new hot wallet into the database
    pub async fn insert_hot_wallet(
        &self,
        address: &str,
        vault: &str,
        secret_id: &str,
        internal_wallet_id: &Uuid,
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        let entry = HotWallet::new(
            secret_id.to_string(),
            vault.to_string(),
            address.to_string(),
            *internal_wallet_id,
        );
        diesel::insert_into(hot_wallets::table)
            .values(entry)
            .execute(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))?;

        Ok(())
    }

    /// Get a hot wallet by its address
    pub async fn get_hot_wallet_by_address(
        &self,
        address: &str,
    ) -> Result<HotWallet, FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        hot_wallets::table
            .filter(hot_wallets::address.eq(address))
            .first::<HotWallet>(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))
    }

    /// Get a hot wallet for the given vault
    pub async fn get_hot_wallet_by_vault(
        &self,
        vault: &str,
    ) -> Result<HotWallet, FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        hot_wallets::table
            .filter(hot_wallets::vault.eq(vault))
            .first::<HotWallet>(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))
    }
}
