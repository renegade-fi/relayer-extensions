//! Interface methods for interacting with the public intents table

use alloy::primitives::B256;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use uuid::Uuid;

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

    /// Update a public intent record
    pub async fn update_public_intent(
        &self,
        public_intent: PublicIntentStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let public_intent_model: PublicIntentModel = public_intent.into();

        diesel::update(public_intents::table)
            .filter(public_intents::intent_hash.eq(public_intent_model.intent_hash.clone()))
            .set(public_intent_model)
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

    /// Get all of a user's active public intent state objects
    pub async fn get_account_active_public_intents(
        &self,
        account_id: Uuid,
        conn: &mut DbConn,
    ) -> Result<Vec<PublicIntentStateObject>, DbError> {
        public_intents::table
            .filter(public_intents::account_id.eq(account_id))
            .filter(public_intents::active.eq(true))
            .load(conn)
            .await
            .map_err(DbError::from)
            .map(|public_intents| public_intents.into_iter().map(PublicIntentModel::into).collect())
    }
}
