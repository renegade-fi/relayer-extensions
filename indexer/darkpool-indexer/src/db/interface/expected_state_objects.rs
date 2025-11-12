//! Interface methods for interacting with the expected state objects table

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::ExpectedStateObjectModel,
        schema::expected_state_objects,
        utils::scalar_to_bigdecimal,
    },
    types::ExpectedStateObject,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a new expected state object record
    pub async fn insert_expected_state_object(
        &self,
        expected_state_object: ExpectedStateObject,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let expected_state_object_model: ExpectedStateObjectModel = expected_state_object.into();

        diesel::insert_into(expected_state_objects::table)
            .values(expected_state_object_model)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get the expected state object record for a given recovery ID, if one
    /// exists
    pub async fn get_expected_state_object(
        &self,
        recovery_id: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<Option<ExpectedStateObject>, DbError> {
        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);

        expected_state_objects::table
            .filter(expected_state_objects::recovery_id.eq(recovery_id_bigdecimal))
            .first(conn)
            .await
            .optional()
            .map_err(DbError::query)
            .map(|maybe_record| maybe_record.map(ExpectedStateObjectModel::into))
    }
}
