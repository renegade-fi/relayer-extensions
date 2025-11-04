//! Interface methods for interacting with the master view seeds table

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::MasterViewSeed,
    schema::master_view_seeds,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a new master view seed
    pub async fn insert_master_view_seed(
        &self,
        account_id: Uuid,
        owner_address: String,
        seed: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let seed_bigdecimal = scalar_to_bigdecimal(seed);
        let master_view_seed = MasterViewSeed { account_id, owner_address, seed: seed_bigdecimal };

        diesel::insert_into(master_view_seeds::table)
            .values(master_view_seed)
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
        conn: &mut DbConn<'_>,
    ) -> Result<MasterViewSeed, DbError> {
        master_view_seeds::table
            .filter(master_view_seeds::account_id.eq(account_id))
            .first(conn)
            .await
            .map_err(DbError::query)
    }
}
