//! API types for the Coinbase connection

use serde::{Deserialize, Serialize};

/// The Coinbase order book snapshot response
#[derive(Debug, Deserialize, Serialize)]
pub struct CoinbaseOrderBookSnapshotResponse {
    /// The bid price levels, sorted in ascending order
    pub bids: Vec<CoinbaseOrderBookLevel>,
    /// The offer price levels, sorted in ascending order
    pub asks: Vec<CoinbaseOrderBookLevel>,
}

/// A tuple of (price, quantity, num_orders)
pub type CoinbaseOrderBookLevel = (String, String, u64);
