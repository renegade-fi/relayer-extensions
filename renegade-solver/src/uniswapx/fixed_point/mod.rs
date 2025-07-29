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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mul_div_down_basic_functionality() {
        // Basic test: 10 * 3 / 2 = 15
        let result = mul_div_down(U256::from(10), U256::from(3), U256::from(2));
        assert_eq!(result.unwrap(), U256::from(15));

        // Test with remainder (rounds down): 10 * 3 / 4 = 7.5 -> 7
        let result = mul_div_down(U256::from(10), U256::from(3), U256::from(4));
        assert_eq!(result.unwrap(), U256::from(7));

        // Test with large numbers
        let result = mul_div_down(U256::from(1000000), U256::from(500000), U256::from(250000));
        assert_eq!(result.unwrap(), U256::from(2000000));
    }

    #[test]
    fn test_mul_div_up_basic_functionality() {
        // Basic test: 10 * 3 / 2 = 15
        let result = mul_div_up(U256::from(10), U256::from(3), U256::from(2));
        assert_eq!(result.unwrap(), U256::from(15));

        // Test with remainder (rounds up): 10 * 3 / 4 = 7.5 -> 8
        let result = mul_div_up(U256::from(10), U256::from(3), U256::from(4));
        assert_eq!(result.unwrap(), U256::from(8));

        // Test with large numbers
        let result = mul_div_up(U256::from(1000000), U256::from(500000), U256::from(250000));
        assert_eq!(result.unwrap(), U256::from(2000000));
    }

    #[test]
    fn test_rounding_behavior_difference() {
        // Case where down and up should differ
        let x = U256::from(7);
        let y = U256::from(3);
        let denominator = U256::from(2);
        // 7 * 3 / 2 = 21 / 2 = 10.5

        let down_result = mul_div_down(x, y, denominator).unwrap();
        let up_result = mul_div_up(x, y, denominator).unwrap();

        assert_eq!(down_result, U256::from(10)); // rounds down
        assert_eq!(up_result, U256::from(11)); // rounds up
        assert!(up_result > down_result);

        // Case where both should be equal (no remainder)
        let result_down = mul_div_down(U256::from(6), U256::from(4), U256::from(3)).unwrap();
        let result_up = mul_div_up(U256::from(6), U256::from(4), U256::from(3)).unwrap();
        assert_eq!(result_down, U256::from(8));
        assert_eq!(result_up, U256::from(8));
    }

    #[test]
    fn test_division_by_zero_error() {
        let result = mul_div_down(U256::from(10), U256::from(5), U256::ZERO);
        assert!(matches!(result, Err(FixedPointMathError::DivisionByZero)));

        let result = mul_div_up(U256::from(10), U256::from(5), U256::ZERO);
        assert!(matches!(result, Err(FixedPointMathError::DivisionByZero)));
    }

    #[test]
    fn test_zero_input_handling() {
        // x = 0 should return 0
        let result = mul_div_down(U256::ZERO, U256::from(5), U256::from(3));
        assert_eq!(result.unwrap(), U256::ZERO);

        let result = mul_div_up(U256::ZERO, U256::from(5), U256::from(3));
        assert_eq!(result.unwrap(), U256::ZERO);

        // y = 0 should return 0
        let result = mul_div_down(U256::from(5), U256::ZERO, U256::from(3));
        assert_eq!(result.unwrap(), U256::ZERO);

        let result = mul_div_up(U256::from(5), U256::ZERO, U256::from(3));
        assert_eq!(result.unwrap(), U256::ZERO);
    }

    #[test]
    fn test_overflow_protection() {
        // Test case that would overflow: U256::MAX * 2
        let result = mul_div_down(U256::MAX, U256::from(2), U256::from(1));
        assert!(matches!(result, Err(FixedPointMathError::Overflow)));

        let result = mul_div_up(U256::MAX, U256::from(2), U256::from(1));
        assert!(matches!(result, Err(FixedPointMathError::Overflow)));

        // Test edge case: max_x calculation
        let large_y = U256::from(1000);
        let max_x = U256::MAX / large_y;
        let result = mul_div_down(max_x + U256::from(1), large_y, U256::from(1));
        assert!(matches!(result, Err(FixedPointMathError::Overflow)));
    }

    #[test]
    fn test_boundary_values() {
        // Test with U256::MAX as denominator (should work)
        let result = mul_div_down(U256::from(100), U256::from(200), U256::MAX);
        assert_eq!(result.unwrap(), U256::ZERO); // Very small result rounds down to 0

        // Test near-maximum safe values
        let result = mul_div_down(U256::MAX, U256::from(1), U256::MAX);
        assert_eq!(result.unwrap(), U256::from(1));

        let result = mul_div_up(U256::MAX, U256::from(1), U256::MAX);
        assert_eq!(result.unwrap(), U256::from(1));
    }
}
