//! Interface methods for interacting with the expected nullifiers table

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::ExpectedNullifier,
    schema::expected_nullifiers,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a new expected nullifier record
    pub async fn insert_expected_nullifier(
        &self,
        nullifier: Scalar,
        account_id: Uuid,
        owner_address: String,
        identifier_seed: Scalar,
        encryption_seed: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);
        let encryption_seed_bigdecimal = scalar_to_bigdecimal(encryption_seed);

        let expected_nullifier = ExpectedNullifier {
            nullifier: nullifier_bigdecimal,
            account_id,
            owner_address,
            identifier_seed: identifier_seed_bigdecimal,
            encryption_seed: encryption_seed_bigdecimal,
        };

        diesel::insert_into(expected_nullifiers::table)
            .values(expected_nullifier)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get the expected nullifier record for a given nullifier
    pub async fn get_expected_nullifier(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<ExpectedNullifier, DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        expected_nullifiers::table
            .filter(expected_nullifiers::nullifier.eq(nullifier_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
    }
}
