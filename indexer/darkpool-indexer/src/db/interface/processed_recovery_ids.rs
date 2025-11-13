//! Interface methods for interacting with the processed recovery IDs table

use bigdecimal::BigDecimal;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::ProcessedRecoveryIDModel,
    schema::processed_recovery_ids,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a recovery ID as processed at the given block number
    pub async fn mark_recovery_id_processed(
        &self,
        recovery_id: Scalar,
        block_number: u64,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);
        let block_number_bigdecimal = BigDecimal::from(block_number);

        let processed_recovery_id = ProcessedRecoveryIDModel {
            recovery_id: recovery_id_bigdecimal,
            block_number: block_number_bigdecimal,
        };

        diesel::insert_into(processed_recovery_ids::table)
            .values(processed_recovery_id)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Check if a recovery ID has been processed
    pub async fn check_recovery_id_processed(
        &self,
        recovery_id: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<bool, DbError> {
        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);

        match processed_recovery_ids::table
            .filter(processed_recovery_ids::recovery_id.eq(recovery_id_bigdecimal))
            .first::<ProcessedRecoveryIDModel>(conn)
            .await
            .optional()
        {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(DbError::query(e)),
        }
    }
}
