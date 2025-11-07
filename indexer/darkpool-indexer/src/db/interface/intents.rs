//! Interface methods for interacting with the intents table

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::IntentModel,
    schema::intents,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert an intent record representing a newly-created intent
    #[allow(clippy::too_many_arguments)]
    pub async fn create_intent(
        &self,
        recovery_stream_seed: Scalar,
        account_id: Uuid,
        input_mint: String,
        output_mint: String,
        owner_address: String,
        min_price: Scalar,
        input_amount: Scalar,
        matching_pool: String,
        allow_external_matches: bool,
        min_fill_size: Scalar,
        precompute_cancellation_proof: bool,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);
        let min_price_bigdecimal = scalar_to_bigdecimal(min_price);
        let input_amount_bigdecimal = scalar_to_bigdecimal(input_amount);
        let min_fill_size_bigdecimal = scalar_to_bigdecimal(min_fill_size);

        let intent = IntentModel {
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            account_id,
            active: true,
            input_mint,
            output_mint,
            owner_address,
            min_price: min_price_bigdecimal,
            input_amount: input_amount_bigdecimal,
            matching_pool,
            allow_external_matches,
            min_fill_size: min_fill_size_bigdecimal,
            precompute_cancellation_proof,
        };

        diesel::insert_into(intents::table)
            .values(intent)
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
        conn: &mut DbConn<'_>,
    ) -> Result<IntentModel, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        intents::table
            .filter(intents::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
    }
}
