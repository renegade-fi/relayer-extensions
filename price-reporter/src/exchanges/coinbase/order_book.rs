//! A local copy of the Coinbase order book

use std::sync::Arc;

use crossbeam_skiplist::SkipSet;
use ordered_float::NotNan;

// ------------------
// | Orderbook Data |
// ------------------

/// A non-nan f64
type NonNanF64 = NotNan<f64>;
/// A shared skip set of price levels
pub type OrderBookLevels = Arc<SkipSet<NonNanF64>>;

/// The order book data stored locally by the connection
#[derive(Clone, Default)]
pub struct CoinbaseOrderBookData {
    /// The bid price levels, sorted in ascending order
    bids: OrderBookLevels,
    /// The offer price levels, sorted in ascending order  
    offers: OrderBookLevels,
}

impl CoinbaseOrderBookData {
    /// Construct a new order book data
    pub fn new() -> Self {
        let bids = Arc::new(SkipSet::new());
        let offers = Arc::new(SkipSet::new());
        Self { bids, offers }
    }

    // ------------------------
    // | Midpoint Calculation |
    // ------------------------

    /// Get the best bid price from the current order book
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.back().map(|e| e.value().into_inner())
    }

    /// Get the best offer price from the current order book
    pub fn best_offer(&self) -> Option<f64> {
        self.offers.front().map(|e| e.value().into_inner())
    }

    /// Get the midpoint price from the current order book
    pub fn midpoint(&self) -> Option<f64> {
        let best_bid = self.best_bid()?;
        let best_offer = self.best_offer()?;
        Some((best_bid + best_offer) / 2.)
    }

    // ----------------------
    // | Order Book Updates |
    // ----------------------

    /// Remove a bid at the given price level
    pub fn remove_bid(&self, price_level: f64) {
        if let Ok(price_notnan) = NotNan::new(price_level) {
            self.bids.remove(&price_notnan);
        }
    }

    /// Remove an offer at the given price level
    pub fn remove_offer(&self, price_level: f64) {
        if let Ok(price_notnan) = NotNan::new(price_level) {
            self.offers.remove(&price_notnan);
        }
    }

    /// Add a bid at the given price level
    pub fn add_bid(&self, price_level: f64) {
        if let Ok(price_notnan) = NotNan::new(price_level) {
            self.bids.insert(price_notnan);
        }
    }

    /// Add an offer at the given price level
    pub fn add_offer(&self, price_level: f64) {
        if let Ok(price_notnan) = NotNan::new(price_level) {
            self.offers.insert(price_notnan);
        }
    }

    /// Clear the order book
    pub fn clear(&self) {
        self.bids.clear();
        self.offers.clear();
    }
}
