//! Quote source implementations for price comparison metrics

pub mod odos;

use renegade_api::http::external_match::AtomicMatchApiBundle;
use renegade_circuit_types::{order::OrderSide, Amount};
use renegade_common::types::token::Token;

use crate::{error::AuthServerError, server::gas_estimation::constants::ESTIMATED_L2_GAS};

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
    /// The fee taken by the source, in units of the received token
    pub fee_take: Amount,
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

    /// Calculates output value with gas costs deducted
    pub fn output_net_of_gas(&self, usdc_per_gas: f64, side: OrderSide) -> f64 {
        let (amount, token) = self.get_receive_amount_mint(side);
        let value = token.convert_to_decimal(amount);
        self.deduct_gas(value, side, usdc_per_gas)
    }

    /// Calculates output value with fees deducted
    pub fn output_net_of_fee(&self, side: OrderSide) -> f64 {
        let (amount, token) = self.get_receive_amount_mint(side);
        self.deduct_fees(amount, &token)
    }

    /// Calculates output value with gas and fees deducted
    pub fn output_net_of_gas_and_fee(&self, side: OrderSide, usdc_per_gas: f64) -> f64 {
        let value = self.output_net_of_fee(side);
        self.deduct_gas(value, side, usdc_per_gas)
    }

    /// Helper to apply gas cost deduction based on order side
    fn deduct_gas(&self, value: f64, side: OrderSide, usdc_per_gas: f64) -> f64 {
        let gas_cost_usdc = self.gas_cost(usdc_per_gas);
        match side {
            OrderSide::Sell => value - gas_cost_usdc,
            OrderSide::Buy => {
                let usdc_per_base = self.price();
                let gas_cost_in_base = gas_cost_usdc / usdc_per_base;
                value - gas_cost_in_base
            },
        }
    }

    /// Helper to apply fee deduction
    fn deduct_fees(&self, amount: Amount, token: &Token) -> f64 {
        let net_amount = amount - self.fee_take;
        token.convert_to_decimal(net_amount)
    }

    /// Gets the amount and token that would be received based on order side
    fn get_receive_amount_mint(&self, side: OrderSide) -> (Amount, Token) {
        match side {
            OrderSide::Sell => (self.quote_amount, Token::from_addr(&self.quote_mint)),
            OrderSide::Buy => (self.base_amount, Token::from_addr(&self.base_mint)),
        }
    }
}

/// Converts the `AtomicMatchApiBundle` into a `QuoteResponse`.
impl From<&AtomicMatchApiBundle> for QuoteResponse {
    fn from(bundle: &AtomicMatchApiBundle) -> Self {
        let gas = bundle.settlement_tx.gas.unwrap_or(ESTIMATED_L2_GAS);
        let fee_take = bundle.fees.total();
        Self {
            quote_mint: bundle.match_result.quote_mint.clone(),
            base_mint: bundle.match_result.base_mint.clone(),
            quote_amount: bundle.match_result.quote_amount,
            base_amount: bundle.match_result.base_amount,
            gas,
            name: RENEGADE_SOURCE_NAME.to_string(),
            fee_take,
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
    ) -> Result<QuoteResponse, AuthServerError> {
        match self {
            QuoteSource::Odos(source) => source
                .get_quote(base_token, quote_token, side, amount)
                .await
                .map_err(AuthServerError::quote_comparison),
        }
    }
}

impl QuoteSource {
    /// Creates a new quote source for the Odos API
    pub fn odos(config: odos::OdosConfig) -> Self {
        QuoteSource::Odos(odos::OdosQuoteSource::new(config))
    }

    /// Creates a new quote source for the Odos API with default configuration
    pub fn odos_default() -> Self {
        Self::odos(odos::OdosConfig::default())
    }
}
