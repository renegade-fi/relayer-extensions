//! Interface methods for interacting with the generic state objects table

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::GenericStateObjectModel,
        schema::generic_state_objects,
        utils::scalar_to_bigdecimal,
    },
    types::GenericStateObject,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a generic state object record representing a newly-created state
    /// object
    #[allow(clippy::too_many_arguments)]
    pub async fn create_generic_state_object(
        &self,
        generic_state_object: GenericStateObject,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let generic_state_object_model: GenericStateObjectModel = generic_state_object.into();

        diesel::insert_into(generic_state_objects::table)
            .values(generic_state_object_model)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get a generic state object by its recovery stream seed
    pub async fn get_generic_state_object(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<GenericStateObject, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        generic_state_objects::table
            .filter(generic_state_objects::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
            .map(GenericStateObjectModel::into)
    }
}
