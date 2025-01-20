//! Defines the quote comparison struct and handler

pub mod handler;
pub mod price_reporter_client;

use renegade_circuit_types::order::OrderSide;

use super::sources::QuoteResponse;

/// Multiplier to convert decimal to basis points (1 basis point = 0.01%)
const DECIMAL_TO_BPS: f64 = 10_000.0;

/// Calculate the difference between two values in basis points.
///
/// Formula: ((value - baseline) / baseline) * 10000
///
/// Returns the relative difference in basis points. A positive result means
/// `value` is higher than the `baseline` by that many basis points.
fn calc_bps_diff(value: f64, baseline: f64) -> f64 {
    let diff_ratio = (value - baseline) / baseline;
    diff_ratio * DECIMAL_TO_BPS
}

/// Represents a single quote comparison between quotes from different sources
pub struct QuoteComparison<'a> {
    /// Our quote
    pub our_quote: &'a QuoteResponse,
    /// The quote from the source
    pub source_quote: &'a QuoteResponse,
    /// USDC per unit of gas
    pub usdc_per_gas: f64,
}

impl<'a> QuoteComparison<'a> {
    /// Calculate the price difference in basis points (bps).
    /// Positive bps indicates a better quote for the given side:
    /// - Sell: our price > source price
    /// - Buy: source price > our price
    pub fn price_diff_bps(&self, side: OrderSide) -> f64 {
        let our_price = self.our_quote.price();
        let source_price = self.source_quote.price();
        match side {
            OrderSide::Sell => calc_bps_diff(our_price, source_price),
            OrderSide::Buy => calc_bps_diff(source_price, our_price),
        }
    }

    /// Calculate the output value net of gas difference in basis points (bps).
    /// Positive bps indicates our quote is better.
    pub fn output_value_net_of_gas_diff_bps(&self, usdc_per_gas: f64, side: OrderSide) -> f64 {
        let our_output_value_net_of_gas = self.our_quote.output_net_of_gas(usdc_per_gas, side);
        let source_output_value_net_of_gas =
            self.source_quote.output_net_of_gas(usdc_per_gas, side);

        calc_bps_diff(our_output_value_net_of_gas, source_output_value_net_of_gas)
    }

    /// Calculate the output value net of fee difference in basis points (bps).
    /// Positive bps indicates our quote is better.
    pub fn output_value_net_of_fee_diff_bps(&self, side: OrderSide) -> f64 {
        let our_output_value_net_of_fee = self.our_quote.output_net_of_fee(side);
        let source_output_value_net_of_fee = self.source_quote.output_net_of_fee(side);

        calc_bps_diff(our_output_value_net_of_fee, source_output_value_net_of_fee)
    }
}
