//! Interface methods for interacting with the processed public intent updates
//! table

use alloy::primitives::{B256, TxHash};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, upsert::excluded};
use diesel_async::RunQueryDsl;

use crate::db::{
    SINGLETON_ROW_ID,
    client::{DbClient, DbConn},
    error::DbError,
    models::{LastIndexedPublicIntentUpdateBlockModel, ProcessedPublicIntentUpdateModel},
    schema::{last_indexed_public_intent_update_block, processed_public_intent_updates},
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a public intent update as processed at the given block number
    ///
    /// If `is_backfill` is true, only the idempotency guard is updated;
    /// the last-indexed block is not advanced.
    pub async fn mark_public_intent_update_processed(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
        block_number: u64,
        is_backfill: bool,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let intent_hash_string = intent_hash.to_string();
        let tx_hash_string = tx_hash.to_string();

        let processed_public_intent_update = ProcessedPublicIntentUpdateModel {
            intent_hash: intent_hash_string,
            tx_hash: tx_hash_string,
        };

        diesel::insert_into(processed_public_intent_updates::table)
            .values(processed_public_intent_update)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        if !is_backfill {
            let block_number_i64 = block_number as i64;
            let record = LastIndexedPublicIntentUpdateBlockModel {
                id: SINGLETON_ROW_ID,
                block_number: block_number_i64,
            };

            diesel::insert_into(last_indexed_public_intent_update_block::table)
                .values(&record)
                .on_conflict(last_indexed_public_intent_update_block::id)
                .do_update()
                .set(
                    last_indexed_public_intent_update_block::block_number
                        .eq(excluded(last_indexed_public_intent_update_block::block_number)),
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

    /// Check if a public intent update has been processed
    pub async fn check_public_intent_update_processed(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let intent_hash_string = intent_hash.to_string();
        let tx_hash_string = tx_hash.to_string();

        processed_public_intent_updates::table
            .filter(processed_public_intent_updates::intent_hash.eq(intent_hash_string))
            .filter(processed_public_intent_updates::tx_hash.eq(tx_hash_string))
            .first::<ProcessedPublicIntentUpdateModel>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.is_some())
    }

    /// Get the latest processed public intent update block number, if one
    /// exists
    pub async fn get_latest_processed_public_intent_update_block(
        &self,
        conn: &mut DbConn,
    ) -> Result<Option<u64>, DbError> {
        last_indexed_public_intent_update_block::table
            .filter(last_indexed_public_intent_update_block::id.eq(SINGLETON_ROW_ID))
            .select(last_indexed_public_intent_update_block::block_number)
            .first::<i64>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(|block_number| block_number as u64))
    }
}
