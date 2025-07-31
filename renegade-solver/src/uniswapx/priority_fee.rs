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

use crate::uniswapx::abis::priority_order::MPS;

/// Compute the priority fee in wei
pub fn compute_priority_fee(priority_order_price: f64, renegade_price: f64, is_sell: bool) -> U256 {
    // Check if we can provide improvement over the order minimum
    let improvement = if is_sell {
        renegade_price >= priority_order_price
    } else {
        renegade_price <= priority_order_price
    };

    if !improvement {
        tracing::info!(
            "Renegade price: {} | UniswapX price: {} | Improvement: 0 bps",
            renegade_price,
            priority_order_price,
        );
        return U256::ZERO;
    }

    // Calculate improvement as a percentage of the order minimum price
    let abs_diff = (renegade_price - priority_order_price).abs();
    let improvement_percent = abs_diff / priority_order_price;

    // Convert to milli-bps
    let priority_fee_mps = improvement_percent * (MPS as f64);

    let priority_fee_wei = U256::from(priority_fee_mps as u128);
    tracing::info!(
        "Renegade price: {} | UniswapX price: {} | Improvement: {} bps | Priority fee: {} wei",
        renegade_price,
        priority_order_price,
        improvement_percent * 10000.0,
        priority_fee_mps
    );

    priority_fee_wei
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_priority_fee_calculation_docs_example() {
        // Example from UniswapX docs:
        // Alice sells 1 ETH for minimum 1000 USDC
        // Fair market rate: 1100 USDC per ETH
        // Filler offers 1090 USDC (10% margin from 100 USDC profit)
        // Expected: 900 bps improvement = 900,000 mps = 900,000 wei

        let priority_order_price = 1000.0; // minimum 1000 USDC
        let renegade_price = 1090.0; // filler's offered price
        let is_sell = true; // selling ETH for USDC

        let priority_fee_wei = compute_priority_fee(priority_order_price, renegade_price, is_sell);

        // Expected calculation:
        // improvement = (1090 - 1000) / 1000 = 0.09 = 9%
        // bps = 0.09 * 10,000 = 900 bps
        // mps = 900 * 1000 = 900,000 mps
        assert_eq!(priority_fee_wei, U256::from(900_000u128));
    }
}
