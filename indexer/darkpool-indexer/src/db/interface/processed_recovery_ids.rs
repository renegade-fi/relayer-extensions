//! Interface methods for interacting with the processed recovery IDs table

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, upsert::excluded};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::db::{
    SINGLETON_ROW_ID,
    client::{DbClient, DbConn},
    error::DbError,
    models::{LastIndexedRecoveryIdBlockModel, ProcessedRecoveryIDModel},
    schema::{last_indexed_recovery_id_block, processed_recovery_ids},
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a recovery ID as processed at the given block number
    ///
    /// If `is_backfill` is true, only the idempotency guard is updated;
    /// the last-indexed block is not advanced.
    pub async fn mark_recovery_id_processed(
        &self,
        recovery_id: Scalar,
        block_number: u64,
        is_backfill: bool,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);

        let processed_recovery_id =
            ProcessedRecoveryIDModel { recovery_id: recovery_id_bigdecimal };

        diesel::insert_into(processed_recovery_ids::table)
            .values(processed_recovery_id)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        if !is_backfill {
            let block_number_i64 = block_number as i64;
            let record = LastIndexedRecoveryIdBlockModel {
                id: SINGLETON_ROW_ID,
                block_number: block_number_i64,
            };

            diesel::insert_into(last_indexed_recovery_id_block::table)
                .values(&record)
                .on_conflict(last_indexed_recovery_id_block::id)
                .do_update()
                .set(
                    last_indexed_recovery_id_block::block_number
                        .eq(excluded(last_indexed_recovery_id_block::block_number)),
                )
                .execute(conn)
                .await
                .map_err(DbError::from)?;
        }

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
        last_indexed_recovery_id_block::table
            .filter(last_indexed_recovery_id_block::id.eq(SINGLETON_ROW_ID))
            .select(last_indexed_recovery_id_block::block_number)
            .first::<i64>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(|block_number| block_number as u64))
    }
}
