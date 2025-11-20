//! Interface methods for interacting with the intents table

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::IntentModel,
        schema::intents,
        utils::scalar_to_bigdecimal,
    },
    types::IntentStateObject,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert an intent record representing a newly-created intent
    pub async fn create_intent(
        &self,
        intent: IntentStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let intent_model: IntentModel = intent.into();

        diesel::insert_into(intents::table)
            .values(intent_model)
            .execute(conn)
            .await
            .map_err(DbError::from)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get an intent by its nullifier
    pub async fn get_intent_by_nullifier(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn,
    ) -> Result<IntentStateObject, DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        intents::table
            .filter(intents::nullifier.eq(nullifier_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::from)
            .map(IntentModel::into)
    }

    /// Get an intent by its recovery stream seed
    pub async fn get_intent_by_recovery_stream_seed(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn,
    ) -> Result<Option<IntentStateObject>, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        intents::table
            .filter(intents::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.map(IntentModel::into))
    }

    /// Check if an intent record exists for a given recovery stream seed
    pub async fn intent_exists(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        intents::table
            .filter(intents::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first::<IntentModel>(conn)
            .await
            .optional()
            .map_err(DbError::from)
            .map(|maybe_record| maybe_record.is_some())
    }
}
