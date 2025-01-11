mod client;
mod error;
mod types;

use super::{QuoteResponse, QuoteSource};
use error::OdosError;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;
use std::result::Result;
use types::OdosQuoteResponse;

use client::{OdosClient, OdosConfig};

// -------------
// | Constants |
// -------------

/// Identifier for this quote source
const NAME: &str = "odos";

// ----------
// | Source |
// ----------

/// Implementation of the Odos quote source for fetching price quotes
#[derive(Debug, Clone)]
pub struct OdosQuoteSource {
    /// Identifier for this quote source
    name: &'static str,
    /// HTTP client for Odos API
    client: OdosClient,
}

impl OdosQuoteSource {
    /// Creates a new OdosQuoteSource instance with default configuration
    pub fn new() -> QuoteSource {
        Self::with_config(OdosConfig::default())
    }

    /// Creates a new OdosQuoteSource instance with custom configuration
    pub fn with_config(config: OdosConfig) -> QuoteSource {
        QuoteSource::Odos(Self { name: NAME, client: OdosClient::new(config) })
    }

    /// Returns the name of this quote source
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Fetches a price quote for a token pair
    pub async fn get_quote(
        &self,
        base_token: Token,
        quote_token: Token,
        side: OrderSide,
        amount: u128,
    ) -> QuoteResponse {
        let (in_token, out_token) = match side {
            OrderSide::Buy => {
                (quote_token.get_addr().to_string(), base_token.get_addr().to_string())
            },
            OrderSide::Sell => {
                (base_token.get_addr().to_string(), quote_token.get_addr().to_string())
            },
        };

        let quote = self
            .client
            .get_quote(&in_token, amount, &out_token)
            .await
            .expect("Failed to get quote from Odos");

        let price =
            calculate_price_from_quote(&quote, side).expect("Failed to calculate price from quote");

        QuoteResponse { price }
    }
}

// ------------
// | Helpers |
// ------------

/// Calculates the effective price from an Odos quote response by taking the
/// ratio of input to output amounts adjusted for their respective token
/// decimals. For buys, price is in_amount/out_amount (quote/base),
/// for sells price is out_amount/in_amount (quote/base).
fn calculate_price_from_quote(
    quote: &OdosQuoteResponse,
    side: OrderSide,
) -> Result<f64, OdosError> {
    let in_amount = quote.get_in_amount()?;
    let out_amount = quote.get_out_amount()?;

    let in_token = Token::from_addr(&quote.in_tokens[0]);
    let out_token = Token::from_addr(&quote.out_tokens[0]);

    let in_amount_decimal = in_token.convert_to_decimal(in_amount);
    let out_amount_decimal = out_token.convert_to_decimal(out_amount);

    Ok(match side {
        OrderSide::Buy => in_amount_decimal / out_amount_decimal,
        OrderSide::Sell => out_amount_decimal / in_amount_decimal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Token constants for testing
    const WETH_ADDRESS: &str = "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1";
    const WETH_DECIMALS: u32 = 18;
    const USDC_ADDRESS: &str = "0xFF970A61A04b1cA14834A43f5dE4533eBDDB5CC8";
    const USDC_DECIMALS: u32 = 6;
    const BASE_AMOUNT: u128 = 1; // 1 unit of token

    /// Helper to convert human readable amount to token amount with decimals
    fn to_token_amount(amount: u128, decimals: u32) -> u128 {
        amount * 10u128.pow(decimals)
    }

    /// Integration test that fetches real quotes from Odos API
    #[tokio::test]
    #[ignore]
    async fn test_fetch_real_quotes() {
        let source = OdosQuoteSource::new();

        // Test buy quote (buying WETH with USDC)

        let buy_amount = to_token_amount(BASE_AMOUNT * 1800, USDC_DECIMALS); // Amount in USDC
        let buy_response = source
            .get_quote(
                Token::from_addr(WETH_ADDRESS),
                Token::from_addr(USDC_ADDRESS),
                OrderSide::Buy,
                buy_amount,
            )
            .await;
        assert!(buy_response.price > 0.0);

        // Test sell quote (selling WETH for USDC)
        let sell_amount = to_token_amount(BASE_AMOUNT, WETH_DECIMALS); // Amount in WETH
        let sell_response = source
            .get_quote(
                Token::from_addr(WETH_ADDRESS),
                Token::from_addr(USDC_ADDRESS),
                OrderSide::Sell,
                sell_amount,
            )
            .await;
        assert!(sell_response.price > 0.0);
    }
}
