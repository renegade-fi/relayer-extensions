//! Extensions for PriorityOrder, PriorityInput, and PriorityOutput
//!
//! Scaling logic is copied from UniswapX's [`PriorityFeeLib.sol` library](https://github.com/Uniswap/UniswapX/blob/4c01ff07cb16df7ad7f5c2b8e9253005c1259275/src/lib/PriorityFeeLib.sol).
use std::cmp::min;

use alloy_primitives::U256;
use renegade_common::types::token::Token;
use renegade_sdk::{
    types::{ExternalOrder, OrderSide},
    ExternalOrderBuilder,
};

use super::{conversion::u256_to_u128, uniswapx::PriorityOrderReactor::PriorityOrder};
use crate::{
    error::SolverResult,
    uniswapx::{
        abis::uniswapx::PriorityOrderReactor::{PriorityInput, PriorityOutput},
        fixed_point::{
            error::{FixedPointMathError, FixedPointResult},
            mul_div_down, mul_div_up,
        },
        NATIVE_ETH_ADDRESS, NATIVE_ETH_ADDRESS_RENEGADE, WETH_TICKER,
    },
};

impl PriorityOrder {
    /// Determines if this is a sell order (output token is USDC)
    pub fn is_sell(&self) -> bool {
        let usdc_address = Token::usdc().get_alloy_address();
        self.output_token().get_alloy_address() == usdc_address
    }

    /// Returns the quote token address (always USDC)
    pub fn quote_token(&self) -> Token {
        if self.is_sell() {
            self.output_token()
        } else {
            Token::from_addr(&self.input.token.to_string())
        }
    }

    /// Returns the unscaled quote amount (USDC amount)
    pub fn quote_amount(&self) -> U256 {
        if self.is_sell() {
            self.total_output_amount()
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
    ///
    /// Maps Native ETH to WETH since this returns a Renegade Token where native
    /// ETH address is not valid
    pub fn base_token(&self) -> Token {
        if self.is_sell() {
            let base = self.map_native_eth_to_weth(self.input.token.to_string());
            Token::from_addr(&base)
        } else {
            let base = self.map_native_eth_to_weth(self.output_token().get_addr().to_string());
            Token::from_addr(&base)
        }
    }

    /// Returns the unscaled base amount (non-USDC token amount)
    pub fn base_amount(&self) -> U256 {
        if self.is_sell() {
            self.input.amount
        } else {
            self.total_output_amount()
        }
    }

    /// Returns the decimal corrected base amount
    pub fn base_amt_decimal_corrected(&self) -> SolverResult<f64> {
        let amount = u256_to_u128(self.base_amount())?;
        let decimal_corrected_amt = self.base_token().convert_to_decimal(amount);
        Ok(decimal_corrected_amt)
    }

    /// Returns the decimal corrected price in quote per base
    pub fn get_price(&self) -> SolverResult<f64> {
        let quote_amt = self.quote_amt_decimal_corrected()?;
        let base_amt = self.base_amt_decimal_corrected()?;
        Ok(quote_amt / base_amt)
    }

    /// Returns `true` if the input amount changes once a priority fee is
    /// applied
    pub fn is_input_scaled(&self) -> bool {
        !self.input.mpsPerPriorityFeeWei.is_zero()
    }

    /// Returns `true` if the aggregate output amount changes once a priority
    /// fee is applied
    pub fn is_output_scaled(&self) -> bool {
        self.outputs.iter().any(|o| !o.mpsPerPriorityFeeWei.is_zero())
    }

    /// Returns `true` if the base side scales
    fn is_base_scaled(&self) -> bool {
        if self.is_sell() {
            self.is_input_scaled()
        } else {
            self.is_output_scaled()
        }
    }

    /// Returns the amount that stays invariant after scaling. If neither side
    /// scales, the output amount is returned.
    ///
    /// Assumes only one side was scaled
    fn invariant_amount(&self) -> U256 {
        if self.is_output_scaled() {
            self.input.amount
        } else {
            self.total_output_amount()
        }
    }

    /// Returns aggregate unscaled output amount
    ///
    /// Assumes all outputs have the same token
    pub fn total_output_amount(&self) -> U256 {
        self.outputs.iter().map(|o| o.amount).sum()
    }

    /// Returns the output token address
    ///
    /// Assumes all outputs have the same token
    pub fn output_token(&self) -> Token {
        Token::from_addr(&self.outputs[0].token.to_string())
    }

    /// Returns the auction start block
    pub fn auction_start_block(&self) -> U256 {
        min(self.auctionStartBlock, self.cosignerData.auctionTargetBlock)
    }

    /// Map Native ETH to WETH
    fn map_native_eth_to_weth(&self, token: String) -> String {
        match token.as_str() {
            NATIVE_ETH_ADDRESS => Token::from_ticker(WETH_TICKER).get_addr(),
            _ => token,
        }
    }

    /// Map Native ETH to Renegade ETH
    ///
    /// Renegade uses a canonical address for native ETH, which is different
    /// from the zero address used by UniswapX.
    fn map_native_eth_to_renegade_eth(&self, token: String) -> String {
        if token == NATIVE_ETH_ADDRESS {
            NATIVE_ETH_ADDRESS_RENEGADE.to_string()
        } else {
            token
        }
    }

    /// Build an ExternalOrder for this PriorityOrder (pre-scaling)
    ///
    /// Chooses between `exact_base_output` and `exact_quote_output` based on
    /// which side will remain invariant when scaling is later applied. If
    /// neither side scales, the quote amount is treated as invariant.
    pub fn to_external_order(&self) -> SolverResult<ExternalOrder> {
        let is_sell = self.is_sell();
        let base_token = self.base_token();
        let quote_token = self.quote_token();
        // Map UniswapX native ETH address to Renegade native ETH address
        let base_mint = self.map_native_eth_to_renegade_eth(base_token.get_addr());
        let quote_mint = quote_token.get_addr();

        let builder = ExternalOrderBuilder::new()
            .base_mint(&base_mint)
            .quote_mint(&quote_mint)
            .side(if is_sell { OrderSide::Sell } else { OrderSide::Buy });

        // Determine amount of invariant side
        let invariant_amount = u256_to_u128(self.invariant_amount())?;

        // Build order with exact invariant amount
        let order = if self.is_base_scaled() {
            builder.exact_quote_output(invariant_amount).build()?
        } else {
            builder.exact_base_output(invariant_amount).build()?
        };
        Ok(order)
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
    /// This implements scaling favoring the swapper where higher priority fees
    /// result in lower required input amounts. The scaling is bounded by 0
    /// and the original input amount.
    pub fn scale(&self, priority_fee_wei: U256) -> FixedPointResult {
        let priority_fee_scaling_rate = self.mpsPerPriorityFeeWei;

        // Discount scaling factor = priority fee * priority fee scaling rate
        let discount_scaling_factor = priority_fee_wei
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
    /// This implements scaling favoring the swapper where higher priority fees
    /// result in higher output amounts. The scaling is bounded by the
    /// original output amount and above.
    pub fn scale(&self, priority_fee_wei: U256) -> FixedPointResult {
        let priority_fee_scaling_rate = self.mpsPerPriorityFeeWei;

        if priority_fee_scaling_rate == U256::ZERO {
            return Ok(self.amount);
        }

        // Scaling factor = priority fee * priority fee scaling rate
        let scaling_factor = priority_fee_wei
            .checked_mul(priority_fee_scaling_rate)
            .ok_or(FixedPointMathError::Overflow)?;

        // Scaling numerator = MPS + scaling factor
        let scaling_numerator =
            MPS_U256.checked_add(scaling_factor).ok_or(FixedPointMathError::Overflow)?;

        let amount = mul_div_up(self.amount, scaling_numerator, MPS_U256)?;

        Ok(amount)
    }
}
