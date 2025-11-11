//! Interface methods for interacting with the balances table

use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_circuit_types::balance::Balance;
use renegade_constants::Scalar;

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
        models::{BalanceCoreChangeset, BalanceModel},
        schema::balances,
        utils::scalar_to_bigdecimal,
    },
    types::BalanceStateObject,
};

impl DbClient {
    // -----------
    // | Setters |
    // -----------

    /// Insert a balance record representing a newly-created balance
    pub async fn create_balance(
        &self,
        balance: BalanceStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let balance_model: BalanceModel = balance.into();

        diesel::insert_into(balances::table)
            .values(balance_model)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    /// Update the core fields of a balance from the corresponding circuit type
    pub async fn update_balance_core(
        &self,
        recovery_stream_seed: Scalar,
        balance: Balance,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);
        let balance_core_changeset: BalanceCoreChangeset = balance.into();

        diesel::update(balances::table)
            .filter(balances::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .set(balance_core_changeset)
            .execute(conn)
            .await
            .map_err(DbError::query)?;

        Ok(())
    }

    // -----------
    // | Getters |
    // -----------

    /// Get a balance by its recovery stream seed
    pub async fn get_balance(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn,
    ) -> Result<BalanceStateObject, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        balances::table
            .filter(balances::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first(conn)
            .await
            .map_err(DbError::query)
            .map(BalanceModel::into)
    }

    /// Check if a balance record exists for a given recovery stream seed
    pub async fn balance_exists(
        &self,
        recovery_stream_seed: Scalar,
        conn: &mut DbConn,
    ) -> Result<bool, DbError> {
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);

        match balances::table
            .filter(balances::recovery_stream_seed.eq(recovery_stream_seed_bigdecimal))
            .first::<BalanceModel>(conn)
            .await
            .optional()
        {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(DbError::query(e)),
        }
    }
}
