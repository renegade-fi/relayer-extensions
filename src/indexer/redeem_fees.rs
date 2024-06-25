//! Fee redemption logic

use std::collections::HashMap;

use tracing::{info, warn};

use crate::helpers::get_binance_price;
use crate::Indexer;

/// The maximum number of fees to redeem in a given run of the indexer
pub(crate) const MAX_FEES_REDEEMED: usize = 20;

impl Indexer {
    /// Redeem the most valuable open fees
    pub async fn redeem_fees(&mut self) -> Result<(), String> {
        info!("redeeming fees...");

        // Get all mints that have unredeemed fees
        let mints = self.get_unredeemed_fee_mints()?;

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
        let most_valuable_fees = self.get_most_valuable_fees(prices)?;

        // TODO: Filter by those fees whose present value exceeds the expected gas costs to redeem
        for fee in most_valuable_fees.into_iter() {
            self.get_or_create_wallet(&fee.mint)?;
        }

        Ok(())
    }

    /// Find or create a wallet to store balances of a given mint
    fn get_or_create_wallet(&mut self, mint: &str) -> Result<(), String> {
        let maybe_wallet = self.get_wallet_for_mint(mint)?;
        let maybe_wallet =
            maybe_wallet.or_else(|| self.find_wallet_with_empty_balance().ok().flatten());

        if maybe_wallet.is_none() {
            println!("creating new wallet for {mint}");
        }

        Ok(())
    }
}
