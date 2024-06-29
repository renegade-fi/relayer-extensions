//! Groups query logic for the indexer

use std::collections::HashMap;

use bigdecimal::BigDecimal;
use diesel::define_sql_function;
use diesel::deserialize::Queryable;
use diesel::deserialize::QueryableByName;
use diesel::sql_query;
use diesel::sql_types::SingleValue;
use diesel::sql_types::{Array, Integer, Nullable, Numeric, Text};
use diesel::PgArrayExpressionMethods;
use diesel::{ExpressionMethods, QueryDsl, RunQueryDsl};
use renegade_constants::MAX_BALANCES;
use renegade_util::raw_err_str;

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
use crate::Indexer;

use super::redeem_fees::MAX_FEES_REDEEMED;

/// The metadata key for the last indexed block
pub(crate) const LAST_INDEXED_BLOCK_KEY: &str = "latest_block";

// Define the `array_length` function
define_sql_function! {
    /// Calculate the length of an array
    fn array_length<T>(array: Array<T>, dim: Integer) -> Nullable<Integer>;
}

define_sql_function! {
    /// Coalesce a nullable value with a default value
    fn coalesce<T: SingleValue>(x: Nullable<T>, y: T) -> T;
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
    pub(crate) fn get_latest_block(&mut self) -> Result<u64, String> {
        let entry = metadata_table
            .filter(metadata_key.eq(LAST_INDEXED_BLOCK_KEY))
            .limit(1)
            .load(&mut self.db_conn)
            .map(|res: Vec<Metadata>| res[0].clone())
            .map_err(raw_err_str!("failed to query latest block: {}"))?;

        entry.value.parse::<u64>().map_err(raw_err_str!("failed to parse latest block: {}"))
    }

    /// Update the latest block number
    pub(crate) fn update_latest_block(&mut self, block_number: u64) -> Result<(), String> {
        let block_string = block_number.to_string();
        diesel::update(metadata_table.find(LAST_INDEXED_BLOCK_KEY))
            .set(metadata_value.eq(block_string))
            .execute(&mut self.db_conn)
            .map_err(raw_err_str!("failed to update latest block: {}"))
            .map(|_| ())
    }

    // --------------
    // | Fees Table |
    // --------------

    /// Insert a fee into the fees table
    pub(crate) fn insert_fee(&mut self, fee: NewFee) -> Result<(), String> {
        diesel::insert_into(fees_table)
            .values(vec![fee])
            .execute(&mut self.db_conn)
            .map_err(raw_err_str!("failed to insert fee: {}"))
            .map(|_| ())
    }

    /// Get all mints that have unredeemed fees
    pub(crate) fn get_unredeemed_fee_mints(&mut self) -> Result<Vec<String>, String> {
        let mints = fees_table
            .select(mint_col)
            .filter(redeemed_col.eq(false))
            .distinct()
            .load(&mut self.db_conn)
            .map_err(raw_err_str!("failed to query unredeemed fees: {}"))?;

        Ok(mints)
    }

    /// Mark a fee as redeemed
    pub(crate) fn mark_fee_as_redeemed(&mut self, tx_hash: &str) -> Result<(), String> {
        let filter = tx_hash_col.eq(tx_hash);
        diesel::update(fees_table.filter(filter))
            .set(redeemed_col.eq(true))
            .execute(&mut self.db_conn)
            .map_err(raw_err_str!("failed to mark fee as redeemed: {}"))
            .map(|_| ())
    }

    /// Get the most valuable fees to be redeemed
    ///
    /// Returns the tx hashes of the most valuable fees to be redeemed
    pub(crate) fn get_most_valuable_fees(
        &mut self,
        prices: HashMap<String, f64>,
        receiver: &str,
    ) -> Result<Vec<FeeValue>, String> {
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
        query_string.push_str("SELECT tx_hash, mint, ");
        query_string.push_str("CASE ");

        // Add the cases
        for (mint, price) in prices.into_iter() {
            query_string.push_str(&format!("WHEN mint = '{}' then amount * {} ", mint, price));
        }
        query_string.push_str("ELSE 0 END as value ");
        query_string
            .push_str(&format!("FROM fees WHERE redeemed = false and receiver = '{}'", receiver));

        // Sort and limit
        query_string.push_str(&format!("ORDER BY value DESC LIMIT {};", MAX_FEES_REDEEMED));

        // Query for the tx hashes
        sql_query(query_string)
            .load(&mut self.db_conn)
            .map_err(raw_err_str!("failed to query most valuable fees: {}"))
    }

    // -----------------
    // | Wallets Table |
    // -----------------

    /// Get the wallet managing an mint, if it exists
    ///
    /// Returns the id and secret id of the wallet
    pub(crate) fn get_wallet_for_mint(
        &mut self,
        mint: &str,
    ) -> Result<Option<WalletMetadata>, String> {
        let wallets: Vec<WalletMetadata> = wallet_table
            .filter(managed_mints_col.contains(vec![mint]))
            .load(&mut self.db_conn)
            .map_err(raw_err_str!("failed to query wallet for mint: {}"))?;

        Ok(wallets.first().cloned())
    }

    /// Find a wallet with an empty balance slot, if one exists
    pub(crate) fn find_wallet_with_empty_balance(
        &mut self,
    ) -> Result<Option<WalletMetadata>, String> {
        let n_mints = coalesce(array_length(managed_mints_col, 1 /* dim */), 0);
        let wallets = wallet_table
            .filter(n_mints.lt(MAX_BALANCES as i32))
            .load(&mut self.db_conn)
            .map_err(raw_err_str!("failed to query wallets with empty balances: {}"))?;

        Ok(wallets.first().cloned())
    }

    /// Insert a new wallet into the wallets table
    pub(crate) fn insert_wallet(&mut self, wallet: WalletMetadata) -> Result<(), String> {
        diesel::insert_into(wallet_table)
            .values(vec![wallet])
            .execute(&mut self.db_conn)
            .map_err(raw_err_str!("failed to insert wallet: {}"))
            .map(|_| ())
    }
}
