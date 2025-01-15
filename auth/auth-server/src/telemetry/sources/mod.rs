//! Quote source implementations for price comparison metrics

mod http_utils;
pub mod odos;

use renegade_api::http::external_match::AtomicMatchApiBundle;
use renegade_circuit_types::{order::OrderSide, Amount};
use renegade_common::types::token::Token;

/// A quote response containing price data
#[derive(Debug)]
pub struct QuoteResponse {
    /// The mint of the quote token in the matched asset pair
    pub quote_mint: String,
    /// The mint of the base token in the matched asset pair
    pub base_mint: String,
    /// The amount of the quote token exchanged by the match
    pub quote_amount: Amount,
    /// The amount of the base token exchanged by the match
    pub base_amount: Amount,
}

/// Converts the `AtomicMatchApiBundle` into a `QuoteResponse`.
impl From<&AtomicMatchApiBundle> for QuoteResponse {
    fn from(bundle: &AtomicMatchApiBundle) -> Self {
        Self {
            quote_mint: bundle.match_result.quote_mint.clone(),
            base_mint: bundle.match_result.base_mint.clone(),
            quote_amount: bundle.match_result.quote_amount,
            base_amount: bundle.match_result.base_amount,
        }
    }
}

impl QuoteResponse {
    /// Calculates the price from the quote and base amounts.
    pub fn price(&self) -> f64 {
        let base_token = Token::from_addr(&self.base_mint);
        let quote_token = Token::from_addr(&self.quote_mint);

        let base_amt = base_token.convert_to_decimal(self.base_amount);
        let quote_amt = quote_token.convert_to_decimal(self.quote_amount);

        quote_amt / base_amt
    }
}

/// Enum representing different types of quote sources
#[derive(Clone)]
pub enum QuoteSource {
    /// The quote source for the Odos API
    Odos(odos::OdosQuoteSource),
}

impl QuoteSource {
    /// Returns the name of the quote source.
    pub fn name(&self) -> &'static str {
        match self {
            QuoteSource::Odos(source) => source.name(),
        }
    }

    /// Asynchronously retrieves a quote for the given base and quote tokens,
    /// order side, and amount.
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
