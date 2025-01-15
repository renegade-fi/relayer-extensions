//! Defines the quote comparison struct and handler
pub mod handler;

use renegade_circuit_types::order::OrderSide;

use super::sources::QuoteResponse;

/// Multiplier to convert decimal to basis points (1 basis point = 0.01%)
const DECIMAL_TO_BPS: f64 = 10_000.0;

/// Represents a single quote comparison between quotes from different sources
pub struct QuoteComparison<'a> {
    pub our_quote: &'a QuoteResponse,
    pub source_quote: &'a QuoteResponse,
    pub source_name: &'a str,
}

impl<'a> QuoteComparison<'a> {
    /// Calculate the price difference in basis points (bps).
    /// Positive bps indicates a better quote for the given side:
    /// - Sell: our price > source price
    /// - Buy: source price > our price
    pub fn price_diff_bps(&self, side: OrderSide) -> i32 {
        let our_price = self.our_quote.price();
        let source_price = self.source_quote.price();
        let price_diff_ratio = match side {
            OrderSide::Sell => (our_price - source_price) / source_price,
            OrderSide::Buy => (source_price - our_price) / our_price,
        };

        (price_diff_ratio * DECIMAL_TO_BPS) as i32
    }
}
