//! Groups query logic for the indexer

use std::collections::HashMap;

use bigdecimal::BigDecimal;
use diesel::deserialize::Queryable;
use diesel::deserialize::QueryableByName;
use diesel::result::Error as DieselError;
use diesel::sql_function;
use diesel::sql_query;
use diesel::sql_types::SingleValue;
use diesel::sql_types::{Array, Integer, Nullable, Numeric, Text};
use diesel::PgArrayExpressionMethods;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use renegade_common::types::wallet::WalletIdentifier;
use renegade_constants::MAX_BALANCES;
use tracing::warn;

use crate::db::models::WalletMetadata;
use crate::db::models::{Metadata, NewFee};
use crate::db::schema::{
    fees::dsl::{
        fees as fees_table, mint as mint_col, redeemed as redeemed_col, tx_hash as tx_hash_col,
    },
    indexing_metadata::dsl::{
        indexing_metadata as metadata_table, key as metadata_key, value as metadata_value,
    },
    wallets::dsl::{mints as managed_mints_col, wallets as wallet_table},
};
use crate::error::FundsManagerError;
use crate::Indexer;

use super::redeem_fees::MAX_FEES_REDEEMED;

/// The metadata key for the last indexed block
pub(crate) const LAST_INDEXED_BLOCK_KEY: &str = "latest_block";

// Define the `array_length` function
sql_function! {
    /// Calculate the length of an array
    fn array_length<T>(array: Array<T>, dim: Integer) -> Nullable<Integer>;
}

sql_function! {
    /// Coalesce a nullable value with a default value
    fn coalesce<T: SingleValue>(x: Nullable<T>, y: T) -> T;
}

sql_function! {
    /// Append an element to an array
    fn array_append<T: SingleValue>(arr: Array<T>, elem: T) -> Array<T>;
}

// ---------------
// | Query Types |
// ---------------

/// A sub-query of the most valuable fees to be redeemed
#[derive(Debug, Queryable, QueryableByName)]
pub(crate) struct FeeValue {
    /// The tx hash of the fee
    #[sql_type = "Text"]
    pub tx_hash: String,
    /// The mint of the fee
    #[sql_type = "Text"]
    pub mint: String,
    /// The receiver of the mint
    #[sql_type = "Text"]
    pub receiver: String,
    /// The value of the fee
    #[sql_type = "Numeric"]
    #[allow(unused)]
    pub value: BigDecimal,
}

// -------------------------
// | Query Implementations |
// -------------------------

impl Indexer {
    // ------------------
    // | Metadata Table |
    // ------------------

    /// Get the latest block number
    pub(crate) async fn get_latest_block(&mut self) -> Result<u64, FundsManagerError> {
        let entry = metadata_table
            .filter(metadata_key.eq(LAST_INDEXED_BLOCK_KEY))
            .limit(1)
            .load::<Metadata>(&mut self.db_conn)
            .await
            .map(|res| res[0].clone())
            .map_err(|_| FundsManagerError::db("failed to query latest block"))?;

        entry
            .value
            .parse::<u64>()
            .map_err(|_| FundsManagerError::db("could not parse latest block"))
    }

    /// Update the latest block number
    pub(crate) async fn update_latest_block(
        &mut self,
        block_number: u64,
    ) -> Result<(), FundsManagerError> {
        let block_string = block_number.to_string();
        diesel::update(metadata_table.find(LAST_INDEXED_BLOCK_KEY))
            .set(metadata_value.eq(block_string))
            .execute(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to update latest block"))
            .map(|_| ())
    }

    // --------------
    // | Fees Table |
    // --------------

    /// Insert a fee into the fees table
    pub(crate) async fn insert_fee(&mut self, fee: NewFee) -> Result<(), FundsManagerError> {
        match diesel::insert_into(fees_table).values(vec![fee]).execute(&mut self.db_conn).await {
            Ok(_) => Ok(()),
            Err(DieselError::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            )) => {
                warn!("Fee already exists in the database, skipping insertion...",);
                Ok(())
            },
            Err(e) => Err(FundsManagerError::db(format!("failed to insert fee: {e}"))),
        }
    }

    /// Get all mints that have unredeemed fees
    pub(crate) async fn get_unredeemed_fee_mints(
        &mut self,
    ) -> Result<Vec<String>, FundsManagerError> {
        let mints = fees_table
            .select(mint_col)
            .filter(redeemed_col.eq(false))
            .distinct()
            .load::<String>(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query unredeemed fees"))?;

        Ok(mints)
    }

    /// Mark a fee as redeemed
    pub(crate) async fn mark_fee_as_redeemed(
        &mut self,
        tx_hash: &str,
    ) -> Result<(), FundsManagerError> {
        let filter = tx_hash_col.eq(tx_hash);
        diesel::update(fees_table.filter(filter))
            .set(redeemed_col.eq(true))
            .execute(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to mark fee as redeemed"))
            .map(|_| ())
    }

    /// Get the most valuable fees to be redeemed
    ///
    /// Returns the tx hashes of the most valuable fees to be redeemed
    pub(crate) async fn get_most_valuable_fees(
        &mut self,
        prices: HashMap<String, f64>,
    ) -> Result<Vec<FeeValue>, FundsManagerError> {
        if prices.is_empty() {
            return Ok(vec![]);
        }

        // We query the fees table with a transformation that calculates the value of
        // each fee using the prices passed in. This query looks something like:
        //  SELECT tx_hash, mint, amount,
        //  CASE
        //      WHEN mint = '<mint1>' then amount * <price1>
        //      WHEN mint = '<mint2>' then amount * <price2>
        //      ...
        //      ELSE 0
        //  END as value
        //  FROM fees
        //  ORDER BY value DESC;
        let mut query_string = String::new();
        query_string.push_str("SELECT tx_hash, mint, receiver, ");
        query_string.push_str("CASE ");

        // Add the cases
        for (mint, price) in prices.into_iter() {
            query_string.push_str(&format!("WHEN mint = '{}' then amount * {} ", mint, price));
        }
        query_string.push_str("ELSE 0 END as value ");
        query_string.push_str("FROM fees WHERE redeemed = false ");

        // Sort and limit
        query_string.push_str(&format!("ORDER BY value DESC LIMIT {};", MAX_FEES_REDEEMED));

        // Query for the tx hashes
        sql_query(query_string)
            .load::<FeeValue>(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query most valuable fees"))
    }

    // -----------------
    // | Wallets Table |
    // -----------------

    /// Get all wallets in the table
    pub(crate) async fn get_all_wallets(
        &mut self,
    ) -> Result<Vec<WalletMetadata>, FundsManagerError> {
        let wallets = wallet_table
            .load::<WalletMetadata>(&mut self.db_conn)
            .await
            .map_err(|e| FundsManagerError::db(format!("failed to load wallets: {}", e)))?;
        Ok(wallets)
    }

    /// Get the wallet managing an mint, if it exists
    ///
    /// Returns the id and secret id of the wallet
    pub(crate) async fn get_wallet_for_mint(
        &mut self,
        mint: &str,
    ) -> Result<Option<WalletMetadata>, FundsManagerError> {
        let wallets: Vec<WalletMetadata> = wallet_table
            .filter(managed_mints_col.contains(vec![mint]))
            .load::<WalletMetadata>(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query wallet for mint"))?;

        Ok(wallets.into_iter().next())
    }

    /// Find a wallet with an empty balance slot, if one exists
    pub(crate) async fn find_wallet_with_empty_balance(
        &mut self,
    ) -> Result<Option<WalletMetadata>, FundsManagerError> {
        let n_mints = coalesce(array_length(managed_mints_col, 1 /* dim */), 0);
        let wallets = wallet_table
            .filter(n_mints.lt(MAX_BALANCES as i32))
            .load::<WalletMetadata>(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query wallets with empty balances"))?;

        Ok(wallets.into_iter().next())
    }

    /// Insert a new wallet into the wallets table
    pub(crate) async fn insert_wallet(
        &mut self,
        wallet: WalletMetadata,
    ) -> Result<(), FundsManagerError> {
        diesel::insert_into(wallet_table)
            .values(vec![wallet])
            .execute(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to insert wallet"))
            .map(|_| ())
    }

    /// Add a new mint to a wallet's managed mints
    pub(crate) async fn add_mint_to_wallet(
        &mut self,
        wallet_id: &WalletIdentifier,
        mint: &str,
    ) -> Result<(), FundsManagerError> {
        diesel::update(wallet_table.find(wallet_id))
            .set(managed_mints_col.eq(array_append(managed_mints_col, mint)))
            .execute(&mut self.db_conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to add mint to wallet"))
            .map(|_| ())
    }
}
