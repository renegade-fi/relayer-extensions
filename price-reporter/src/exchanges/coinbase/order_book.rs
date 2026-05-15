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

    /// Add a bid at the given price level.
    ///
    /// Non-positive prices are silently rejected: a zero or negative bid in
    /// the book would skew `midpoint` toward `best_offer / 2`. A real Coinbase
    /// feed never emits these, but partial-book / reset edge cases can.
    pub fn add_bid(&self, price_level: f64) {
        if !(price_level > 0.0) {
            return;
        }
        if let Ok(price_notnan) = NotNan::new(price_level) {
            self.bids.insert(price_notnan);
        }
    }

    /// Add an offer at the given price level.
    ///
    /// Non-positive prices are silently rejected: a zero or negative offer
    /// becomes the new `best_offer` and pulls `midpoint` to `best_bid / 2`.
    /// This is the failure mode behind the cbBTC pricing incident on
    /// 2026-05-08 ~07:10 UTC.
    pub fn add_offer(&self, price_level: f64) {
        if !(price_level > 0.0) {
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduces the cbBTC pricing failure on 2026-05-08 around 07:10 UTC
    /// (00:10 PDT): a real bid plus a zero-priced offer caused midpoint to
    /// return real_bid / 2 (~$39,837 vs real BTC ~$79,674), which propagated
    /// through `Exchange::Renegade` cbBTC pricing into the funds-manager
    /// `swap_execution_cost` metric. Zero-priced levels must not enter the
    /// book at all; with only real-bid data and no real offer, midpoint must
    /// be `None`.
    #[test]
    fn midpoint_ignores_zero_priced_offer() {
        let book = CoinbaseOrderBookData::new();
        book.add_bid(79_674.0);
        book.add_offer(0.0);

        assert_eq!(
            book.midpoint(),
            None,
            "midpoint must not synthesize half of best_bid when the only \
             offer is zero-priced",
        );
    }

    /// Symmetric to the above: a zero-priced bid alongside a real offer must
    /// not produce real_offer / 2.
    #[test]
    fn midpoint_ignores_zero_priced_bid() {
        let book = CoinbaseOrderBookData::new();
        book.add_bid(0.0);
        book.add_offer(79_675.0);

        assert_eq!(book.midpoint(), None);
    }

    /// A zero-priced offer arriving alongside a real offer must not become
    /// the new best offer (which would skew midpoint downward by ~½).
    #[test]
    fn zero_offer_does_not_displace_real_offer() {
        let book = CoinbaseOrderBookData::new();
        book.add_bid(79_674.0);
        book.add_offer(79_675.0);
        book.add_offer(0.0);

        assert_eq!(book.best_offer(), Some(79_675.0));
        assert_eq!(book.midpoint(), Some((79_674.0 + 79_675.0) / 2.0));
    }

    /// Negative prices are nonsensical in an order book and must also be
    /// rejected at insertion time.
    #[test]
    fn negative_prices_are_rejected() {
        let book = CoinbaseOrderBookData::new();
        book.add_bid(-1.0);
        book.add_offer(-1.0);

        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_offer(), None);
    }

    /// Sanity: a normal book still computes the midpoint correctly.
    #[test]
    fn midpoint_normal_book() {
        let book = CoinbaseOrderBookData::new();
        book.add_bid(100.0);
        book.add_bid(99.0);
        book.add_offer(101.0);
        book.add_offer(102.0);

        assert_eq!(book.best_bid(), Some(100.0));
        assert_eq!(book.best_offer(), Some(101.0));
        assert_eq!(book.midpoint(), Some(100.5));
    }
}
