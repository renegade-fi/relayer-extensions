//! Extensions for PriorityOrder
use alloy_primitives::U256;
use renegade_common::types::token::Token;

use super::{conversion::u256_to_u128, uniswapx::PriorityOrderReactor::PriorityOrder};

use crate::error::SolverResult;

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
