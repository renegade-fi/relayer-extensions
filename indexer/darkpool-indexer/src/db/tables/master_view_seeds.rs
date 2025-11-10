//! Interface methods for interacting with the master view seeds table

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::MasterViewSeedModel,
        schema::master_view_seeds,
    },
    types::MasterViewSeed,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a new master view seed
    pub async fn insert_master_view_seed(
        &self,
        master_view_seed: MasterViewSeed,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let master_view_seed_model: MasterViewSeedModel = master_view_seed.into();

        diesel::insert_into(master_view_seeds::table)
            .values(master_view_seed_model)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    /// Update a master view seed
    pub async fn update_master_view_seed(
        &self,
        master_view_seed: MasterViewSeed,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let master_view_seed_model: MasterViewSeedModel = master_view_seed.into();

        diesel::update(master_view_seeds::table)
            .filter(master_view_seeds::account_id.eq(master_view_seed_model.account_id))
            .set(master_view_seed_model)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get the master view seed for a given account
    pub async fn get_account_master_view_seed(
        &self,
        account_id: Uuid,
        conn: &mut DbConn,
    ) -> Result<MasterViewSeed, DbError> {
        master_view_seeds::table
            .filter(master_view_seeds::account_id.eq(account_id))
            .first(conn)
            .await
            .map_err(DbError::query)
            .map(MasterViewSeedModel::into)
    }
}
