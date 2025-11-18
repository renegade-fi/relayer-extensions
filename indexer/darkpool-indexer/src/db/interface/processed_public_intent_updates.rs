//! Interface methods for interacting with the processed public intent updates
//! table

use alloy::primitives::B256;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::ProcessedPublicIntentUpdateModel,
    schema::processed_public_intent_updates,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Mark a public intent update as processed at the given block number
    pub async fn mark_public_intent_update_processed(
        &self,
        intent_hash: B256,
        version: u64,
        block_number: u64,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let intent_hash_string = intent_hash.to_string();
        let version_i64 = version as i64;
        let block_number_i64 = block_number as i64;

        let processed_public_intent_update = ProcessedPublicIntentUpdateModel {
            intent_hash: intent_hash_string,
            version: version_i64,
            block_number: block_number_i64,
        };

        diesel::insert_into(processed_public_intent_updates::table)
            .values(processed_public_intent_update)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Check if a public intent update has been processed
    pub async fn check_public_intent_update_processed(
        &self,
        intent_hash: B256,
        version: u64,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let intent_hash_string = intent_hash.to_string();
        let version_i64 = version as i64;

        processed_public_intent_updates::table
            .filter(processed_public_intent_updates::intent_hash.eq(intent_hash_string))
            .filter(processed_public_intent_updates::version.eq(version_i64))
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
        processed_public_intent_updates::table
            .select(processed_public_intent_updates::block_number)
            .order(processed_public_intent_updates::block_number.desc())
            .first::<i64>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(|block_number| block_number as u64))
    }
}
