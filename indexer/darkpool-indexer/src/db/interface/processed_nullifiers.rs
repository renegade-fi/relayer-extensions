//! Interface methods for interacting with the processed nullifiers table

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, upsert::excluded};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::db::{
    SINGLETON_ROW_ID,
    client::{DbClient, DbConn},
    error::DbError,
    models::{LastIndexedNullifierBlockModel, ProcessedNullifierModel},
    schema::{last_indexed_nullifier_block, processed_nullifiers},
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a nullifier as processed at the given block number
    ///
    /// If `is_backfill` is true, only the idempotency guard is updated;
    /// the last-indexed block is not advanced.
    pub async fn mark_nullifier_processed(
        &self,
        nullifier: Scalar,
        block_number: u64,
        is_backfill: bool,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        let processed_nullifier = ProcessedNullifierModel { nullifier: nullifier_bigdecimal };

        diesel::insert_into(processed_nullifiers::table)
            .values(processed_nullifier)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        if !is_backfill {
            let block_number_i64 = block_number as i64;
            let record = LastIndexedNullifierBlockModel {
                id: SINGLETON_ROW_ID,
                block_number: block_number_i64,
            };

            diesel::insert_into(last_indexed_nullifier_block::table)
                .values(&record)
                .on_conflict(last_indexed_nullifier_block::id)
                .do_update()
                .set(
                    last_indexed_nullifier_block::block_number
                        .eq(excluded(last_indexed_nullifier_block::block_number)),
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

    /// Check if a nullifier has been processed
    pub async fn check_nullifier_processed(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        processed_nullifiers::table
            .filter(processed_nullifiers::nullifier.eq(nullifier_bigdecimal))
            .first::<ProcessedNullifierModel>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.is_some())
    }

    /// Get the latest processed nullifier block number, if one exists
    pub async fn get_latest_processed_nullifier_block(
        &self,
        conn: &mut DbConn,
    ) -> Result<Option<u64>, DbError> {
        last_indexed_nullifier_block::table
            .filter(last_indexed_nullifier_block::id.eq(SINGLETON_ROW_ID))
            .select(last_indexed_nullifier_block::block_number)
            .first::<i64>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(|block_number| block_number as u64))
    }
}
