//! Fee redemption logic

use std::collections::HashMap;

use bigdecimal::BigDecimal;
use diesel::deserialize::Queryable;
use diesel::deserialize::QueryableByName;
use diesel::query_dsl::methods::{DistinctDsl, FilterDsl, SelectDsl};
use diesel::sql_query;
use diesel::sql_types::Numeric;
use diesel::sql_types::Text;
use diesel::{ExpressionMethods, RunQueryDsl};
use renegade_util::raw_err_str;
use tracing::{info, warn};

use crate::db::schema::fees::dsl::{
    fees as fees_table, mint as mint_col, redeemed as redeemed_col,
};
use crate::helpers::get_binance_price;
use crate::Indexer;

/// The maximum number of fees to redeem in a given run of the indexer
const MAX_FEES_REDEEMED: usize = 20;

/// A sub-query of the most valuable fees to be redeemed
#[derive(Debug, Queryable, QueryableByName)]
struct FeeValue {
    /// The tx hash of the fee
    #[sql_type = "Text"]
    tx_hash: String,
    /// The value of the fee
    #[sql_type = "Numeric"]
    value: BigDecimal,
}

impl Indexer {
    /// Redeem the most valuable open fees
    pub async fn redeem_fees(&mut self) -> Result<(), String> {
        info!("redeeming fees...");

        // Get all mints that have unredeemed fees
        let mints = self.get_unredeemed_fee_mints().await?;

        // Get the prices of each redeemable mint, we want to redeem the most profitable fees first
        let mut prices = HashMap::new();
        for mint in mints.into_iter() {
            let maybe_price = get_binance_price(&mint, &self.usdc_mint, &self.relayer_url).await?;
            if let Some(price) = maybe_price {
                prices.insert(mint, price);
            } else {
                warn!("{}: no price", mint);
            }
        }

        // Get the most valuable fees and redeem them
        let most_valuable_fees = self.get_most_valuable_fees(prices).await?;
        println!("most valuable fees:");
        for (i, fee) in most_valuable_fees.into_iter().enumerate() {
            println!("#{i}: {fee:?}");
        }

        Ok(())
    }

    /// Get all mints that have unredeemed fees
    async fn get_unredeemed_fee_mints(&mut self) -> Result<Vec<String>, String> {
        let mints = fees_table
            .select(mint_col)
            .filter(redeemed_col.eq(false))
            .distinct()
            .load(&mut self.db_conn)
            .map_err(raw_err_str!("failed to query unredeemed fees: {}"))?;

        Ok(mints)
    }

    /// Get the most valuable fees to be redeemed
    ///
    /// Returns the tx hashes of the most valuable fees to be redeemed
    async fn get_most_valuable_fees(
        &mut self,
        prices: HashMap<String, f64>,
    ) -> Result<Vec<FeeValue>, String> {
        if prices.is_empty() {
            return Ok(vec![]);
        }

        // We query the fees table with a transformation that calculates the value of each fee using the prices passed in.
        // This query looks something like:
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
        query_string.push_str("SELECT tx_hash, ");
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
            .load(&mut self.db_conn)
            .map_err(raw_err_str!("failed to query most valuable fees: {}"))
    }
}
