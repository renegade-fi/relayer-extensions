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
use renegade_sdk::types::AtomicMatchApiBundle;
use tracing::warn;

use crate::error::SolverResult;
use crate::uniswapx::abis::conversion::u256_to_u128;
use crate::uniswapx::abis::priority_order::MPS;
use crate::uniswapx::abis::uniswapx::PriorityOrderReactor::PriorityOrder;

/// Compute the priority fee based on the improvement the Renegade bundle
/// provides over the worst-cast PriorityOrder
///
/// Returns the priority fee in wei as `U256`.
pub fn compute_priority_fee(
    order: &PriorityOrder,
    bundle: &AtomicMatchApiBundle,
) -> SolverResult<U256> {
    // Determine which side (input or output) scales
    let input_scaled = order.is_input_scaled();
    let output_scaled = order.is_output_scaled();

    // Calculate improvement fraction and get the scaling rate (k)
    let (improvement_fraction, mps_per_priority_fee_wei): (f64, u128) = if input_scaled {
        // Input side scales → expect venue to use *less* input than worst-case
        let order_input_amt = u256_to_u128(order.input.amount)? as f64;
        let venue_input_amt = bundle.send.amount as f64;
        let scaling_rate = u256_to_u128(order.input.mpsPerPriorityFeeWei)?;

        let improvement = calculate_input_improvement(order_input_amt, venue_input_amt);

        (improvement, scaling_rate)
    } else if output_scaled {
        // Output side scales → expect venue to give *more* output than worst-case
        let order_output_amt = u256_to_u128(order.total_output_amount())? as f64;
        let venue_output_amt = bundle.receive.amount as f64;
        let scaling_rate = u256_to_u128(order.outputs[0].mpsPerPriorityFeeWei)?;

        let improvement = calculate_output_improvement(order_output_amt, venue_output_amt);

        (improvement, scaling_rate)
    } else {
        // TODO: Implement bidding strategy for static orders
        warn!("Neither side scales, no priority fee adjustment");
        return Ok(U256::ZERO);
    };

    let priority_fee_wei =
        improvement_to_priority_fee(improvement_fraction, mps_per_priority_fee_wei);

    Ok(priority_fee_wei)
}

/// Calculate the improvement of the input side
fn calculate_input_improvement(order_amount: f64, venue_amount: f64) -> f64 {
    if venue_amount >= order_amount {
        return 0.0;
    }

    1.0 - (venue_amount / order_amount)
}

/// Calculate the improvement of the output side
fn calculate_output_improvement(order_amount: f64, venue_amount: f64) -> f64 {
    if venue_amount <= order_amount {
        return 0.0;
    }

    (venue_amount / order_amount) - 1.0
}

/// Convert improvement fraction to milli-bps then divide by the scaling rate
/// Formula: priority_fee_wei = (improvement_fraction * MPS) /
/// mps_per_priority_fee_wei
fn improvement_to_priority_fee(improvement: f64, scaling_rate: u128) -> U256 {
    let priority_fee_wei_f64 = improvement * (MPS as f64) / (scaling_rate as f64);
    let priority_fee_wei = priority_fee_wei_f64.floor() as u128;

    U256::from(priority_fee_wei)
}

#[cfg(test)]
mod tests {
    use super::{calculate_output_improvement, improvement_to_priority_fee};
    use alloy_primitives::U256;

    // Example from [docs](https://docs.uniswap.org/contracts/uniswapx/guides/priority/priorityorderreactor#example-implementation)
    // Alice sells 1 ETH, min 1000 USDC. Desired execution: 1090 USDC
    // => improvement = 900 bps = 0.09 = 900_000 mps
    // With 1 mps per wei, priority fee = 900_000 wei.
    #[test]
    fn docs_example_output_scaled_priority_fee() {
        let order_output_usdc = 1000.0_f64;
        let venue_output_usdc = 1090.0_f64;
        let mps_per_priority_fee_wei: u128 = 1;

        let improvement = calculate_output_improvement(order_output_usdc, venue_output_usdc);
        assert!(
            (improvement - 0.09).abs() < 1e-12,
            "improvement fraction should be 0.09 (900 bps)"
        );

        let priority_fee = improvement_to_priority_fee(improvement, mps_per_priority_fee_wei);
        assert_eq!(priority_fee, U256::from(900_000u128), "priority fee should be 900,000 wei");
    }
}
