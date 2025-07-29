//! Extensions for PriorityOrder, PriorityInput, and PriorityOutput
//!
//! Scaling logic is copied from UniswapX's [`PriorityFeeLib.sol` library](https://github.com/Uniswap/UniswapX/blob/4c01ff07cb16df7ad7f5c2b8e9253005c1259275/src/lib/PriorityFeeLib.sol).
use alloy_primitives::U256;
use renegade_common::types::token::Token;

use super::{conversion::u256_to_u128, uniswapx::PriorityOrderReactor::PriorityOrder};

use crate::{
    error::SolverResult,
    uniswapx::{
        abis::uniswapx::PriorityOrderReactor::{PriorityInput, PriorityOutput},
        fixed_point::{
            error::{FixedPointMathError, FixedPointResult},
            mul_div_down, mul_div_up,
        },
    },
};

impl PriorityOrder {
    /// Determines if this is a buy order (input token is USDC)
    pub fn is_sell(&self) -> bool {
        let usdc_address = Token::usdc().get_alloy_address();
        self.input.token == usdc_address
    }

    /// Returns the quote token address (always USDC)
    pub fn quote_token(&self) -> Token {
        if self.is_sell() {
            Token::from_addr(&self.outputs[0].token.to_string())
        } else {
            Token::from_addr(&self.input.token.to_string())
        }
    }

    /// Returns the quote amount (USDC amount)
    pub fn quote_amount(&self) -> U256 {
        if self.is_sell() {
            self.outputs[0].amount
        } else {
            self.input.amount
        }
    }

    /// Returns the decimal corrected quote amount
    pub fn quote_amt_decimal_corrected(&self) -> SolverResult<f64> {
        let amount = u256_to_u128(self.quote_amount())?;
        let decimal_corrected_amt = self.quote_token().convert_to_decimal(amount);
        Ok(decimal_corrected_amt)
    }

    /// Returns the base token address (non-USDC token)
    pub fn base_token(&self) -> Token {
        if self.is_sell() {
            Token::from_addr(&self.input.token.to_string())
        } else {
            Token::from_addr(&self.outputs[0].token.to_string())
        }
    }

    /// Returns the base amount (non-USDC token amount)
    pub fn base_amount(&self) -> U256 {
        if self.is_sell() {
            self.input.amount
        } else {
            self.outputs[0].amount
        }
    }

    /// Returns the decimal corrected base amount
    pub fn base_amt_decimal_corrected(&self) -> SolverResult<f64> {
        let amount = u256_to_u128(self.base_amount())?;
        let decimal_corrected_amt = self.base_token().convert_to_decimal(amount);
        Ok(decimal_corrected_amt)
    }

    /// Calculate the order price from this PriorityOrder
    ///
    /// Assumes one side of the order is USDC and there is only one output token
    pub fn get_price(&self) -> SolverResult<f64> {
        let quote_decimal_corrected_amt = self.quote_amt_decimal_corrected()?;
        let base_decimal_corrected_amt = self.base_amt_decimal_corrected()?;
        let price = quote_decimal_corrected_amt / base_decimal_corrected_amt;
        Ok(price)
    }
}

/// Milli-basis-point conversion value (10^7 = 100% in milli-bps)
pub(crate) const MPS: u64 = 10_000_000;

/// Milli-basis-point basis represented as a U256 constant
const MPS_U256: U256 = U256::from_limbs([MPS, 0, 0, 0]);

impl PriorityInput {
    /// Returns the input scaled by 1 - (priority_fee ×
    /// priority_fee_scaling_rate)
    ///
    /// This implements favorable scaling where higher priority fees result in
    /// lower required input amounts. The scaling is bounded by 0 and the
    /// original input amount.
    pub fn scale(&self, priority_fee: U256) -> FixedPointResult<U256> {
        let priority_fee_scaling_rate = self.mpsPerPriorityFeeWei;

        // Discount scaling factor = priority fee * priority fee scaling rate
        let discount_scaling_factor = priority_fee
            .checked_mul(priority_fee_scaling_rate)
            .ok_or(FixedPointMathError::Overflow)?;

        if discount_scaling_factor >= MPS_U256 {
            return Ok(U256::ZERO);
        }

        if discount_scaling_factor == U256::ZERO {
            return Ok(self.amount);
        }

        // Scaling numerator = MPS - discount scaling factor
        let scaling_numerator = MPS_U256 - discount_scaling_factor; // Safe due to early return above

        let amount = mul_div_down(self.amount, scaling_numerator, MPS_U256)?;

        Ok(amount)
    }
}

impl PriorityOutput {
    /// Returns the output scaled by 1 + (priority_fee ×
    /// priority_fee_scaling_rate)
    ///
    /// This implements favorable scaling where higher priority fees result in
    /// higher output amounts. The scaling is bounded by the original output
    /// amount and above.
    pub fn scale(&self, priority_fee: U256) -> FixedPointResult<U256> {
        let priority_fee_scaling_rate = self.mpsPerPriorityFeeWei;

        if priority_fee_scaling_rate == U256::ZERO {
            return Ok(self.amount);
        }

        // Scaling factor = priority fee * priority fee scaling rate
        let scaling_factor = priority_fee
            .checked_mul(priority_fee_scaling_rate)
            .ok_or(FixedPointMathError::Overflow)?;

        // Scaling numerator = MPS + scaling factor
        let scaling_numerator =
            MPS_U256.checked_add(scaling_factor).ok_or(FixedPointMathError::Overflow)?;

        let amount = mul_div_up(self.amount, scaling_numerator, MPS_U256)?;

        Ok(amount)
    }
}
