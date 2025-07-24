//! Groups query logic for the indexer

use std::collections::HashMap;

use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::deserialize::Queryable;
use diesel::deserialize::QueryableByName;
use diesel::dsl::sum;
use diesel::result::Error as DieselError;
use diesel::sql_function;
use diesel::sql_query;
use diesel::sql_types::SingleValue;
use diesel::sql_types::{Array, Integer, Nullable, Numeric, Text};
use diesel::ExpressionMethods;
use diesel::PgArrayExpressionMethods;
use diesel::QueryDsl;
use diesel_async::RunQueryDsl;
use renegade_circuit_types::Amount;
use renegade_common::types::wallet::WalletIdentifier;
use renegade_constants::MAX_BALANCES;
use tracing::warn;
use uuid::Uuid;

use crate::db::models::RenegadeWalletMetadata;
use crate::db::models::{Metadata, NewFee};
use crate::db::schema::{fees, indexing_metadata, renegade_wallets};
use crate::error::FundsManagerError;
use crate::helpers::to_env_agnostic_name;
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

    /// Get the latest indexed block number on the chain managed by the Indexer
    pub(crate) async fn get_latest_block(&self) -> Result<u64, FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let entry = indexing_metadata::table
            .find((LAST_INDEXED_BLOCK_KEY, to_env_agnostic_name(self.chain)))
            .first::<Metadata>(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query latest block"))?;

        entry
            .value
            .parse::<u64>()
            .map_err(|_| FundsManagerError::db("could not parse latest block"))
    }

    /// Update the latest indexed block number on the chain managed by the
    /// Indexer
    pub(crate) async fn update_latest_block(
        &self,
        block_number: u64,
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let block_string = block_number.to_string();
        diesel::update(
            indexing_metadata::table
                .find((LAST_INDEXED_BLOCK_KEY, to_env_agnostic_name(self.chain))),
        )
        .set(indexing_metadata::value.eq(block_string))
        .execute(&mut conn)
        .await
        .map_err(|_| FundsManagerError::db("failed to update latest block"))
        .map(|_| ())
    }

    // --------------
    // | Fees Table |
    // --------------

    /// Insert a fee into the fees table
    pub(crate) async fn insert_fee(&self, fee: NewFee) -> Result<(), FundsManagerError> {
        let mut conn = self.get_conn().await?;
        match diesel::insert_into(fees::table).values(vec![fee]).execute(&mut conn).await {
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
    pub(crate) async fn get_unredeemed_fee_mints(&self) -> Result<Vec<String>, FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let mints = fees::table
            .select(fees::mint)
            .filter(fees::redeemed.eq(false))
            .filter(fees::chain.eq(to_env_agnostic_name(self.chain)))
            .distinct()
            .load::<String>(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query unredeemed fees"))?;

        Ok(mints)
    }

    /// Get the total amount of unredeemed fees for each mint
    pub(crate) async fn get_unredeemed_fee_totals(
        &self,
    ) -> Result<Vec<(String, Amount)>, FundsManagerError> {
        let mut conn = self.get_conn().await?;

        let totals = fees::table
            .filter(fees::redeemed.eq(false))
            .filter(fees::chain.eq(to_env_agnostic_name(self.chain)))
            .group_by(fees::mint)
            .select((fees::mint, sum(fees::amount)))
            .load::<(String, Option<BigDecimal>)>(&mut conn)
            .await
            .map_err(|e| {
                FundsManagerError::db(format!("failed to query unredeemed fee totals: {e}"))
            })?;

        let non_null_totals = totals
            .into_iter()
            .filter_map(|(mint, maybe_total)| {
                maybe_total.and_then(|total| total.to_u128()).map(|total_u128| (mint, total_u128))
            })
            .collect();

        Ok(non_null_totals)
    }

    /// Mark a fee as redeemed
    pub(crate) async fn mark_fee_as_redeemed(
        &self,
        tx_hash: &str,
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let filter = fees::tx_hash.eq(tx_hash);
        diesel::update(fees::table.filter(filter))
            .set(fees::redeemed.eq(true))
            .execute(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to mark fee as redeemed"))
            .map(|_| ())
    }

    /// Get the most valuable fees to be redeemed
    ///
    /// Returns the `MAX_FEES_REDEEMED` most valuable fees to be redeemed
    pub(crate) async fn get_most_valuable_fees(
        &self,
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
        //  WHERE redeemed = false AND chain = '<chain>'
        //  ORDER BY value DESC;
        let mut query_string = String::new();
        query_string.push_str("SELECT tx_hash, mint, receiver, ");
        query_string.push_str("CASE ");

        // Add the cases
        for (mint, price) in prices.into_iter() {
            query_string.push_str(&format!("WHEN mint = '{}' then amount * {} ", mint, price));
        }
        query_string.push_str("ELSE 0 END as value ");
        query_string.push_str(&format!(
            "FROM fees WHERE redeemed = false AND chain = '{}' ",
            to_env_agnostic_name(self.chain)
        ));

        // Sort and limit
        query_string.push_str(&format!("ORDER BY value DESC LIMIT {};", MAX_FEES_REDEEMED));

        // Query for the tx hashes
        let mut conn = self.get_conn().await?;
        sql_query(query_string)
            .load::<FeeValue>(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query most valuable fees"))
    }

    // -----------------
    // | Wallets Table |
    // -----------------

    /// Get a wallet by its ID
    pub(crate) async fn get_wallet_by_id(
        &self,
        wallet_id: &Uuid,
    ) -> Result<RenegadeWalletMetadata, FundsManagerError> {
        let mut conn = self.get_conn().await?;
        renegade_wallets::table
            .filter(renegade_wallets::id.eq(wallet_id))
            .filter(renegade_wallets::chain.eq(to_env_agnostic_name(self.chain)))
            .first::<RenegadeWalletMetadata>(&mut conn)
            .await
            .map_err(|e| FundsManagerError::db(format!("failed to get wallet by ID: {}", e)))
    }

    /// Get all wallets in the table on the chain managed by the Indexer
    pub(crate) async fn get_all_wallets(
        &self,
    ) -> Result<Vec<RenegadeWalletMetadata>, FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let wallets = renegade_wallets::table
            .filter(renegade_wallets::chain.eq(to_env_agnostic_name(self.chain)))
            .load::<RenegadeWalletMetadata>(&mut conn)
            .await
            .map_err(|e| FundsManagerError::db(format!("failed to load wallets: {}", e)))?;
        Ok(wallets)
    }

    /// Get the wallet managing a mint, if it exists
    pub(crate) async fn get_wallet_for_mint(
        &self,
        mint: &str,
    ) -> Result<Option<RenegadeWalletMetadata>, FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let wallets: Vec<RenegadeWalletMetadata> = renegade_wallets::table
            .filter(renegade_wallets::mints.contains(vec![mint]))
            .filter(renegade_wallets::chain.eq(to_env_agnostic_name(self.chain)))
            .load::<RenegadeWalletMetadata>(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query wallet for mint"))?;

        Ok(wallets.into_iter().next())
    }

    /// Find a wallet with an empty balance slot, if one exists
    pub(crate) async fn find_wallet_with_empty_balance(
        &self,
    ) -> Result<Option<RenegadeWalletMetadata>, FundsManagerError> {
        let mut conn = self.get_conn().await?;
        let n_mints = coalesce(array_length(renegade_wallets::mints, 1 /* dim */), 0);
        let wallets = renegade_wallets::table
            .filter(n_mints.lt(MAX_BALANCES as i32))
            .filter(renegade_wallets::chain.eq(to_env_agnostic_name(self.chain)))
            .load::<RenegadeWalletMetadata>(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to query wallets with empty balances"))?;

        Ok(wallets.into_iter().next())
    }

    /// Insert a new wallet into the wallets table
    pub(crate) async fn insert_wallet(
        &self,
        wallet: RenegadeWalletMetadata,
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_conn().await?;
        diesel::insert_into(renegade_wallets::table)
            .values(vec![wallet])
            .execute(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to insert wallet"))
            .map(|_| ())
    }

    /// Add a new mint to a wallet's managed mints
    pub(crate) async fn add_mint_to_wallet(
        &self,
        wallet_id: &WalletIdentifier,
        mint: &str,
    ) -> Result<(), FundsManagerError> {
        let mut conn = self.get_conn().await?;
        diesel::update(renegade_wallets::table.find(wallet_id))
            .set(renegade_wallets::mints.eq(array_append(renegade_wallets::mints, mint)))
            .execute(&mut conn)
            .await
            .map_err(|_| FundsManagerError::db("failed to add mint to wallet"))
            .map(|_| ())
    }
}
