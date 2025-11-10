//! Interface methods for interacting with the processed nullifiers table

use bigdecimal::BigDecimal;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::ProcessedNullifierModel,
    schema::processed_nullifiers,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a nullifier as processed at the given block number
    pub async fn mark_nullifier_processed(
        &self,
        nullifier: Scalar,
        block_number: u64,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let block_number_bigdecimal = BigDecimal::from(block_number);

        let processed_nullifier = ProcessedNullifierModel {
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
    pub async fn nullifier_processed(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        match processed_nullifiers::table
            .filter(processed_nullifiers::nullifier.eq(nullifier_bigdecimal))
            .first::<ProcessedNullifierModel>(conn)
            .await
            .optional()
        {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(DbError::query(e)),
        }
    }
}
