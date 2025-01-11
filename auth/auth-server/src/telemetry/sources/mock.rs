use rand::Rng;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

use super::{QuoteResponse, QuoteSource};

/// A mock quote source that generates random prices within 2% of the input
/// price
#[derive(Debug, Clone)]
pub struct MockQuoteSource {
    name: &'static str,
}

impl MockQuoteSource {
    pub fn new(name: &'static str) -> QuoteSource {
        QuoteSource::Mock(Self { name })
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub async fn get_quote(
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
