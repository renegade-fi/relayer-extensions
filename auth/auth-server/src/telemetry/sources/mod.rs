//! Mock quote source for price comparison metrics

use async_trait::async_trait;
use rand::Rng;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

/// A quote response containing price data
#[derive(Debug)]
pub struct QuoteResponse {
    pub base_address: String,
    pub quote_address: String,
    pub price: f64,
}

/// A trait defining the interface for quote sources
#[async_trait]
pub trait QuoteSource: Send + Sync {
    fn name(&self) -> &'static str;

    async fn get_quote(
        &self,
        base_token: Token,
        quote_token: Token,
        side: OrderSide,
        amount: u128,
        our_price: f64,
    ) -> QuoteResponse;
}

/// A mock quote source that generates random prices within 2% of the input
/// price
#[derive(Debug)]
pub struct MockQuoteSource {
    name: &'static str,
}

impl MockQuoteSource {
    pub fn new(name: &'static str) -> Self {
        Self { name }
    }
}

#[async_trait]
impl QuoteSource for MockQuoteSource {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn get_quote(
        &self,
        base_token: Token,
        quote_token: Token,
        _side: OrderSide,
        _amount: u128,
        our_price: f64,
    ) -> QuoteResponse {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let price_diff_percent = rand::thread_rng().gen_range(-0.02..=0.02);
        QuoteResponse {
            base_address: base_token.get_addr().to_string(),
            quote_address: quote_token.get_addr().to_string(),
            price: our_price * (1.0 + price_diff_percent),
        }
    }
}
