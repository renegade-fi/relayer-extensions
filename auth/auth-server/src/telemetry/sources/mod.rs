//! Quote source implementations for price comparison metrics

mod http_utils;
pub mod odos;

use renegade_api::http::external_match::AtomicMatchApiBundle;
use renegade_circuit_types::{order::OrderSide, Amount};
use renegade_common::types::token::Token;

/// The gas estimation to use if fetching a gas estimation fails
/// From https://github.com/renegade-fi/renegade/blob/main/workers/api-server/src/http/external_match.rs/#L62
const DEFAULT_GAS_ESTIMATION: u64 = 4_000_000; // 4m
/// The name of our quote source
const RENEGADE_SOURCE_NAME: &str = "renegade";

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
    /// The gas estimate for a quote
    pub gas: u64,
    /// The name of the source that provided the quote
    pub name: String,
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

    /// Calculates the estimated gas cost in USDC
    pub fn gas_cost(&self, usdc_per_gas: f64) -> f64 {
        let gas_total = self.gas as f64;
        gas_total * usdc_per_gas
    }

    /// Calculates the net output value of a trade, accounting for gas costs.
    pub fn output_value_net_of_gas(&self, usdc_per_gas: f64, side: OrderSide) -> f64 {
        // Get decimal corrected amounts
        let base_token = Token::from_addr(&self.base_mint);
        let quote_token = Token::from_addr(&self.quote_mint);

        let base_amt = base_token.convert_to_decimal(self.base_amount);
        let quote_amt = quote_token.convert_to_decimal(self.quote_amount);

        // Get gas cost in USDC
        let usdc_gas_cost = self.gas_cost(usdc_per_gas);

        // Subtract gas cost from net output value
        match side {
            OrderSide::Sell => quote_amt - usdc_gas_cost,
            OrderSide::Buy => {
                let usdc_per_base = self.price();
                let gas_cost_in_base = usdc_gas_cost / usdc_per_base;
                base_amt - gas_cost_in_base
            },
        }
    }
}

/// Converts the `AtomicMatchApiBundle` into a `QuoteResponse`.
impl From<&AtomicMatchApiBundle> for QuoteResponse {
    fn from(bundle: &AtomicMatchApiBundle) -> Self {
        let gas = bundle.settlement_tx.gas().map_or(DEFAULT_GAS_ESTIMATION, |gas| gas.as_u64());
        Self {
            quote_mint: bundle.match_result.quote_mint.clone(),
            base_mint: bundle.match_result.base_mint.clone(),
            quote_amount: bundle.match_result.quote_amount,
            base_amount: bundle.match_result.base_amount,
            gas,
            name: RENEGADE_SOURCE_NAME.to_string(),
        }
    }
}

/// Enum representing different types of quote sources
#[derive(Clone)]
pub enum QuoteSource {
    /// The quote source for the Odos API
    Odos(odos::OdosQuoteSource),
}

impl QuoteSource {
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

impl QuoteSource {
    pub fn odos(config: odos::OdosConfig) -> Self {
        QuoteSource::Odos(odos::OdosQuoteSource::new(config))
    }

    pub fn odos_default() -> Self {
        Self::odos(odos::OdosConfig::default())
    }
}
