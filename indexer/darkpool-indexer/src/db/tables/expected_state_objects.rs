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
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let expected_state_object_model: ExpectedStateObjectModel = expected_state_object.into();

        diesel::insert_into(expected_state_objects::table)
            .values(expected_state_object_model)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    /// Delete an expected state object record
    pub async fn delete_expected_state_object(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        diesel::delete(expected_state_objects::table)
            .filter(expected_state_objects::nullifier.eq(nullifier_bigdecimal))
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get the expected state object record for a given nullifier, if one
    /// exists
    pub async fn get_expected_state_object(
        &self,
        nullifier: Scalar,
        conn: &mut DbConn,
    ) -> Result<Option<ExpectedStateObject>, DbError> {
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);

        expected_state_objects::table
            .filter(expected_state_objects::nullifier.eq(nullifier_bigdecimal))
            .first(conn)
            .await
            .optional()
            .map_err(DbError::query)
            .map(|maybe_record| maybe_record.map(ExpectedStateObjectModel::into))
    }
}
