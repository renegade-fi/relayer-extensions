//! Miscellaneous utility types and helper functions.

use common::types::{exchange::Exchange, token::Token};
use price_reporter::exchange::PriceStreamType;
use tokio::sync::watch::Sender;

/// A type alias for a tuple of (exchange, base token, quote token)
pub type PairInfo = (Exchange, Token, Token);

/// A type alias for the sender end of a price stream
pub type PriceSender = Sender<PriceStreamType>;
