//! Interface methods for interacting with the processed nullifiers table

use bigdecimal::BigDecimal;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::ProcessedNullifier,
    schema::processed_nullifiers,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a processed nullifier record
    pub async fn insert_processed_nullifier(
        &self,
        nullifier: Scalar,
        block_number: u64,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let block_number_bigdecimal = BigDecimal::from(block_number);

        let processed_nullifier = ProcessedNullifier {
            nullifier: nullifier_bigdecimal,
            block_number: block_number_bigdecimal,
        };

        diesel::insert_into(processed_nullifiers::table)
            .values(processed_nullifier)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Check if a nullifier has been processed
    pub async fn check_nullifier_processed(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<bool, DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        match processed_nullifiers::table
            .filter(processed_nullifiers::nullifier.eq(nullifier_bigdecimal))
            .first::<ProcessedNullifier>(conn)
            .await
        {
            Ok(_) => Ok(true),
            Err(diesel::NotFound) => Ok(false),
            Err(e) => Err(DbError::query(e)),
        }
    }
}
