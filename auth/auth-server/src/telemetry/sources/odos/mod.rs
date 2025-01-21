//! A client for the Odos API

mod client;
mod error;
mod types;

use super::QuoteResponse;
use renegade_circuit_types::{fixed_point::FixedPoint, order::OrderSide};
use renegade_common::types::token::Token;

use client::OdosClient;

pub use client::OdosConfig;
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;

// -------------
// | Constants |
// -------------

/// The name of the Odos quote source
const SOURCE_NAME: &str = "odos";

/// The fee rate for Odos Simple Swap using "volatile tokens".
///
/// This is defined in https://docs.odos.xyz/build/fees#swap-fees
const ODOS_FEE_RATE: f64 = 0.0025; // 0.25%

// ----------
// | Source |
// ----------

/// Implementation of the Odos quote source for fetching price quotes
#[derive(Debug, Clone)]
pub struct OdosQuoteSource {
    /// HTTP client for Odos API
    client: OdosClient,
}

impl OdosQuoteSource {
    /// Creates a new OdosQuoteSource instance with custom configuration
    pub fn new(config: OdosConfig) -> Self {
        Self { client: OdosClient::new(config) }
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
            OrderSide::Buy => (quote_token, base_token),
            OrderSide::Sell => (base_token, quote_token),
        };

        // Fetch quote from Odos
        let quote = self
            .client
            .get_quote(&in_token.get_addr().to_string(), amount, &out_token.get_addr().to_string())
            .await
            .expect("Failed to get quote from Odos");

        // When buying, we input the quote token and receive the base token
        // When selling, we input the base token and receive the quote token
        let (base_mint, quote_mint) = match side {
            OrderSide::Buy => (quote.get_out_token().unwrap(), quote.get_in_token().unwrap()),
            OrderSide::Sell => (quote.get_in_token().unwrap(), quote.get_out_token().unwrap()),
        };

        let (base_amount, quote_amount) = match side {
            OrderSide::Buy => (quote.get_out_amount().unwrap(), quote.get_in_amount().unwrap()),
            OrderSide::Sell => (quote.get_in_amount().unwrap(), quote.get_out_amount().unwrap()),
        };

        // Calculate the fee take
        let fee_rate = FixedPoint::from_f64_round_down(ODOS_FEE_RATE);
        let receive_amount = Scalar::from(quote.get_out_amount().unwrap());
        let fee_take = (fee_rate * receive_amount).floor();

        QuoteResponse {
            base_amount,
            base_mint,
            quote_amount,
            quote_mint,
            gas: quote.gas_estimate as u64,
            name: SOURCE_NAME.to_string(),
            fee_take: scalar_to_u128(&fee_take),
        }
    }
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
        let source = OdosQuoteSource::new(OdosConfig::default());

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
        assert!(buy_response.price() > 0.0);

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
        assert!(sell_response.price() > 0.0);
    }
}
