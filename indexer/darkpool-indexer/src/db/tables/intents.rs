//! Interface methods for interacting with the intents table

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_circuit_types::intent::Intent;
use renegade_constants::Scalar;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::{IntentCoreChangeset, IntentModel},
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
            .map_err(DbError::query)?;

        Ok(())
    }

    /// Update the core fields of an intent from the corresponding circuit type
    pub async fn update_intent_core(
        &self,
        recovery_stream_seed: Scalar,
        intent: Intent,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);
        let intent_core_changeset: IntentCoreChangeset = intent.into();

        diesel::update(intents::table)
            .filter(intents::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .set(intent_core_changeset)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get an intent by its recovery stream seed
    pub async fn get_intent(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn,
    ) -> Result<IntentStateObject, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        intents::table
            .filter(intents::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
            .map(IntentModel::into)
    }

    /// Check if an intent record exists for a given recovery stream seed
    pub async fn intent_exists(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        match intents::table
            .filter(intents::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first::<IntentModel>(conn)
            .await
            .optional()
        {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(DbError::query(e)),
        }
    }
}
