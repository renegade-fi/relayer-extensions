//! Interface methods for interacting with the processed recovery IDs table

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
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);
        let block_number_i64 = block_number as i64;

        let processed_recovery_id = ProcessedRecoveryIDModel {
            recovery_id: recovery_id_bigdecimal,
            block_number: block_number_i64,
        };

        diesel::insert_into(processed_recovery_ids::table)
            .values(processed_recovery_id)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Check if a recovery ID has been processed
    pub async fn check_recovery_id_processed(
        &self,
        recovery_id: Scalar,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);

        processed_recovery_ids::table
            .filter(processed_recovery_ids::recovery_id.eq(recovery_id_bigdecimal))
            .first::<ProcessedRecoveryIDModel>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.is_some())
    }

    /// Get the latest processed recovery ID block number, if one exists
    pub async fn get_latest_processed_recovery_id_block(
        &self,
        conn: &mut DbConn,
    ) -> Result<Option<u64>, DbError> {
        processed_recovery_ids::table
            .select(processed_recovery_ids::block_number)
            .order(processed_recovery_ids::block_number.desc())
            .first::<i64>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(|block_number| block_number as u64))
    }
}
