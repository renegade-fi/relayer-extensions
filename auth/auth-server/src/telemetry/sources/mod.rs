//! Quote source implementations for price comparison metrics

mod http_utils;
pub mod odos;

use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

/// A quote response containing price data
#[derive(Debug)]
pub struct QuoteResponse {
    pub price: f64,
}

/// Enum representing different types of quote sources
#[derive(Clone)]
pub enum QuoteSource {
    Odos(odos::OdosQuoteSource),
}

impl QuoteSource {
    pub fn name(&self) -> &'static str {
        match self {
            QuoteSource::Odos(source) => source.name(),
        }
    }

    pub async fn get_quote(
        &self,
        base_token: Token,
        quote_token: Token,
        side: OrderSide,
        amount: u128,
    ) -> QuoteResponse {
        match self {
            QuoteSource::Odos(source) => {
                source.get_quote(base_token, quote_token, side, amount).await
            },
        }
    }
}
