//! Queries for managing custody data

use diesel_async::RunQueryDsl;
use renegade_util::err_str;

use crate::db::models::HotWallet;
use crate::db::schema::hot_wallets;
use crate::error::FundsManagerError;
use crate::CustodyClient;

impl CustodyClient {
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
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_db_conn().await?;
        let entry = HotWallet::new(secret_id.to_string(), vault.to_string(), address.to_string());
        diesel::insert_into(hot_wallets::table)
            .values(entry)
            .execute(&mut conn)
            .await
            .map_err(err_str!(FundsManagerError::Db))?;

        Ok(())
    }
}