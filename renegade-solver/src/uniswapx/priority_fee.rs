//! Helper methods for computing priority fees
//!
//! We maintain the following invariant from the UniswapX specification:
/// For every wei of priority fee above a certain threshold (an optional value
/// specified in the order), the user is owed 1 milli-bps more of their output
/// token (or less of their input token).
///
/// See [UniswapX's documentation](https://docs.uniswapx.org/docs/priority-fee)
/// for more details.
use alloy_primitives::U256;
use renegade_common::types::token::Token;

use crate::{
    error::SolverResult,
    uniswapx::{
        abis::{conversion::u256_to_u128, uniswapx::PriorityOrderReactor::PriorityOrder},
        UniswapXSolver,
    },
};

/// Multiplier to convert decimal to basis points (1 basis point = 0.01%)
const DECIMAL_TO_BPS: f64 = 10_000.0;
/// Multiplier to convert basis points to milli-bps (1 milli-bps = 0.001%)
const BPS_TO_MPB: f64 = 1000.0;

/// Compute the priority fee in wei
pub fn compute_priority_fee(priority_order_price: f64, renegade_price: f64, is_sell: bool) -> U256 {
    // Check if we can provide improvement over the order minimum
    let no_improvement = (is_sell && renegade_price < priority_order_price)
        || (!is_sell && renegade_price > priority_order_price);

    if no_improvement {
        return U256::ZERO;
    }

    // Calculate improvement as a percentage of the order minimum price
    // Sell: (renegade_price - priority_order_price) / priority_order_price (+1.0)
    // Buy:  (priority_order_price - renegade_price) / priority_order_price (-1.0)
    let improvement_direction = if is_sell { 1.0 } else { -1.0 };
    let improvement_percentage =
        improvement_direction * (renegade_price - priority_order_price) / priority_order_price;

    // Convert to basis points then to milli-bps
    let improvement_bps = improvement_percentage * DECIMAL_TO_BPS;
    let priority_fee_mps = improvement_bps * BPS_TO_MPB;

    U256::from(priority_fee_mps as u128)
}

impl UniswapXSolver {
    /// Get the price of a token from the price reporter client
    ///
    /// Assumes one side of the order is USDC
    pub(crate) async fn get_renegade_price(&self, order: &PriorityOrder) -> SolverResult<f64> {
        let is_buy = self.is_usdc(order.input.token);
        let mint = if is_buy { order.outputs[0].token } else { order.input.token };
        let price = self.price_reporter_client.get_price(&mint.to_string(), self.chain_id).await?;
        Ok(price)
    }

    /// Get the price of a token from a PriorityOrder
    ///
    /// Assumes one side of the order is USDC
    pub(crate) async fn get_priority_order_price(
        &self,
        order: &PriorityOrder,
    ) -> SolverResult<f64> {
        let is_buy = self.is_usdc(order.input.token);

        let quote_amount = if is_buy { order.input.amount } else { order.outputs[0].amount };
        let quote_amount_u128 = u256_to_u128(quote_amount)?;
        let quote_token = Token::new(&self.get_usdc_address().to_string(), self.chain_id);
        let quote_decimal_corrected_amt = quote_token.convert_to_decimal(quote_amount_u128);

        let base_mint = if is_buy { order.outputs[0].token } else { order.input.token };
        let base_amount = if is_buy { order.outputs[0].amount } else { order.input.amount };
        let base_amount_u128 = u256_to_u128(base_amount)?;
        let base_token = Token::new(&base_mint.to_string(), self.chain_id);
        let base_decimal_corrected_amt = base_token.convert_to_decimal(base_amount_u128);

        let price = quote_decimal_corrected_amt / base_decimal_corrected_amt;
        Ok(price)
    }
}
