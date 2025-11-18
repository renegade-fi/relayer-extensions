//! Interface methods for interacting with the public intents table

use alloy::primitives::B256;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::PublicIntentModel,
        schema::public_intents,
    },
    types::PublicIntentStateObject,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a public intent record representing a newly-created public intent
    pub async fn create_public_intent(
        &self,
        public_intent: PublicIntentStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let public_intent_model: PublicIntentModel = public_intent.into();

        diesel::insert_into(public_intents::table)
            .values(public_intent_model)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get a public intent by its hash
    pub async fn get_public_intent_by_hash(
        &self,
        intent_hash: B256,
        conn: &mut DbConn,
    ) -> Result<PublicIntentStateObject, DbError> {
        let intent_hash_string = intent_hash.to_string();

        public_intents::table
            .filter(public_intents::intent_hash.eq(intent_hash_string))
            .first(conn)
            .await
            .map_err(DbError::from)
            .map(PublicIntentModel::into)
    }

    /// Check if a public intent record exists for a given intent hash
    pub async fn public_intent_exists(
        &self,
        intent_hash: B256,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let intent_hash_string = intent_hash.to_string();

        public_intents::table
            .filter(public_intents::intent_hash.eq(intent_hash_string))
            .first::<PublicIntentModel>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.is_some())
    }
}
