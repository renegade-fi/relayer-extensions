//! Emulates Solidity's fixed point math functions found in
//! [FixedPointMathLib](https://github.com/transmissions11/solmate/blob/main/src/utils/FixedPointMathLib.sol)
use crate::uniswapx::fixed_point::error::{FixedPointMathError, FixedPointResult};
use alloy_primitives::U256;

pub mod error;

/// Returns x * y / denominator, rounded down.
/// Equivalent to Solidity's mulDivDown with overflow protection
pub fn mul_div_down(x: U256, y: U256, denominator: U256) -> FixedPointResult<U256> {
    if denominator.is_zero() {
        return Err(FixedPointMathError::DivisionByZero);
    }

    if y.is_zero() {
        return Ok(U256::ZERO);
    }

    // Check for overflow: x <= U256::MAX / y
    let max_x = U256::MAX / y;
    if x > max_x {
        return Err(FixedPointMathError::Overflow);
    }

    // Safe to multiply and divide
    Ok((x * y) / denominator)
}

/// Returns x * y / denominator, rounded up.
/// Equivalent to Solidity's mulDivUp with overflow protection
pub fn mul_div_up(x: U256, y: U256, denominator: U256) -> FixedPointResult<U256> {
    if denominator.is_zero() {
        return Err(FixedPointMathError::DivisionByZero);
    }

    if y.is_zero() {
        return Ok(U256::ZERO);
    }

    // Check for overflow: x <= U256::MAX / y
    let max_x = U256::MAX / y;
    if x > max_x {
        return Err(FixedPointMathError::Overflow);
    }

    let product = x * y;
    let quotient = product / denominator;
    let remainder = product % denominator;

    // Add 1 if there's a remainder (round up)
    if remainder > U256::ZERO {
        quotient.checked_add(U256::from(1)).ok_or(FixedPointMathError::Overflow)
    } else {
        Ok(quotient)
    }
}
