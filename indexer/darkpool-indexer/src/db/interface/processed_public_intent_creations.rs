//! Interface methods for interacting with the processed public intent creations
//! table

use alloy::primitives::{B256, TxHash};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::ProcessedPublicIntentCreationModel,
    schema::processed_public_intent_creations,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a public intent creation as processed at the given block number
    pub async fn mark_public_intent_creation_processed(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
        block_number: u64,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let intent_hash_string = intent_hash.to_string();
        let tx_hash_string = tx_hash.to_string();
        let block_number_i64 = block_number as i64;

        let processed_public_intent_creation = ProcessedPublicIntentCreationModel {
            intent_hash: intent_hash_string,
            tx_hash: tx_hash_string,
            block_number: block_number_i64,
        };

        diesel::insert_into(processed_public_intent_creations::table)
            .values(processed_public_intent_creation)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Check if a public intent creation with the given intent hash and
    /// transaction hash has been processed
    pub async fn check_public_intent_creation_processed(
        &self,
        intent_hash: B256,
        tx_hash: TxHash,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let intent_hash_string = intent_hash.to_string();
        let tx_hash_string = tx_hash.to_string();

        processed_public_intent_creations::table
            .filter(processed_public_intent_creations::intent_hash.eq(intent_hash_string))
            .filter(processed_public_intent_creations::tx_hash.eq(tx_hash_string))
            .first::<ProcessedPublicIntentCreationModel>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.is_some())
    }

    /// Get the latest processed public intent creation block number, if one
    /// exists
    pub async fn get_latest_processed_public_intent_creation_block(
        &self,
        conn: &mut DbConn,
    ) -> Result<Option<u64>, DbError> {
        processed_public_intent_creations::table
            .select(processed_public_intent_creations::block_number)
            .order(processed_public_intent_creations::block_number.desc())
            .first::<i64>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(|block_number| block_number as u64))
    }
}
