//! Interface methods for interacting with the generic state objects table

use bigdecimal::{BigDecimal, One};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::{GenericStateObject, ObjectType},
    schema::generic_state_objects,
    utils::scalar_to_bigdecimal,
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
        identifier_seed: Scalar,
        account_id: Uuid,
        object_type: ObjectType,
        nullifier: Scalar,
        encryption_seed: Scalar,
        owner_address: String,
        public_shares: Vec<Scalar>,
        private_shares: Vec<Scalar>,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let encryption_seed_bigdecimal = scalar_to_bigdecimal(encryption_seed);

        let public_shares_bigdecimal =
            public_shares.into_iter().map(scalar_to_bigdecimal).collect();

        let private_shares_bigdecimal =
            private_shares.into_iter().map(scalar_to_bigdecimal).collect();

        let generic_state_object = GenericStateObject {
            identifier_seed: identifier_seed_bigdecimal,
            account_id,
            active: true,
            object_type,
            nullifier: nullifier_bigdecimal,
            version: BigDecimal::one(),
            encryption_seed: encryption_seed_bigdecimal,
            owner_address,
            public_shares: public_shares_bigdecimal,
            private_shares: private_shares_bigdecimal,
        };

        diesel::insert_into(generic_state_objects::table)
            .values(generic_state_object)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get a generic state object by its identifier seed
    pub async fn get_generic_state_object(
        &self,
        identifier_seed: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<GenericStateObject, DbError> {
        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);

        generic_state_objects::table
            .filter(generic_state_objects::identifier_seed.eq(identifier_seed_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
    }
}
