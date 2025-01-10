//! Mock quote source for price comparison metrics

use rand::Rng;
use renegade_common::types::token::Token;

/// A quote response containing price data
#[derive(Debug, Clone)]
pub struct QuoteResponse {
    pub base_token: Token,
    pub quote_token: Token,
    pub price: f64,
}

/// A mock quote source that generates random prices within 2% of the input
/// price
#[derive(Clone)]
pub struct MockQuoteSource {
    name: &'static str,
}

impl MockQuoteSource {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn get_quote(
        &self,
        base_token: Token,
        quote_token: Token,
        our_price: f64,
    ) -> QuoteResponse {
        let price_diff_percent = rand::thread_rng().gen_range(-0.02..=0.02);
        QuoteResponse { base_token, quote_token, price: our_price * (1.0 + price_diff_percent) }
    }
}
