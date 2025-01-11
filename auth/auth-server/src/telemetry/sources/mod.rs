//! Quote source implementations for price comparison metrics

pub mod mock;

use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

/// A quote response containing price data
#[derive(Debug)]
pub struct QuoteResponse {
    pub base_address: String,
    pub quote_address: String,
    pub price: f64,
}

/// Enum representing different types of quote sources
#[derive(Clone)]
pub enum QuoteSource {
    Mock(mock::MockQuoteSource),
    // Add other quote source types here as needed
}

impl QuoteSource {
    pub fn name(&self) -> &'static str {
        match self {
            QuoteSource::Mock(source) => source.name(),
            // Add other source types here
        }
    }

    pub async fn get_quote(
        &self,
        base_token: Token,
        quote_token: Token,
        side: OrderSide,
        amount: u128,
        our_price: f64,
    ) -> QuoteResponse {
        match self {
            QuoteSource::Mock(source) => {
                source.get_quote(base_token, quote_token, side, amount, our_price).await
            }, // Add other source types here
        }
    }
}
