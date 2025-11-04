//! Interface methods for interacting with the balances table

use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, DbConn},
    error::DbError,
    models::Balance,
    schema::balances,
    utils::scalar_to_bigdecimal,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a balance record representing a newly-created balance
    #[allow(clippy::too_many_arguments)]
    pub async fn create_balance(
        &self,
        identifier_seed: Scalar,
        account_id: Uuid,
        mint: String,
        owner_address: String,
        one_time_key: String,
        protocol_fee: Scalar,
        relayer_fee: Scalar,
        amount: Scalar,
        allow_public_fills: bool,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);
        let protocol_fee_bigdecimal = scalar_to_bigdecimal(protocol_fee);
        let relayer_fee_bigdecimal = scalar_to_bigdecimal(relayer_fee);
        let amount_bigdecimal = scalar_to_bigdecimal(amount);

        let balance = Balance {
            identifier_seed: identifier_seed_bigdecimal,
            account_id,
            active: true,
            mint,
            owner_address,
            one_time_key,
            protocol_fee: protocol_fee_bigdecimal,
            relayer_fee: relayer_fee_bigdecimal,
            amount: amount_bigdecimal,
            allow_public_fills,
        };

        diesel::insert_into(balances::table)
            .values(balance)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get a balance by its identifier seed
    pub async fn get_balance(
        &self,
        identifier_seed: Scalar,
        conn: &mut DbConn<'_>,
    ) -> Result<Balance, DbError> {
        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);

        balances::table
            .filter(balances::identifier_seed.eq(identifier_seed_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
    }
}
