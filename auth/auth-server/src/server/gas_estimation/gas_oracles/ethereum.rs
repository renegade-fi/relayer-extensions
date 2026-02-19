//! Ethereum L1 gas estimation
//!
//! On Ethereum L1, there is no separate L1 data posting cost (unlike L2s).
//! The total gas cost is simply execution gas * gas price.

use alloy::primitives::{Address, U256};
use alloy::providers::{DynProvider, Provider};

use super::GasPriceEstimation;

/// Estimate the gas cost for a transaction on Ethereum L1
///
/// Unlike L2s (Arbitrum/Base), there is no separate L1 data posting cost.
/// The total cost is simply execution gas * gas price.
pub async fn estimate_l1_gas_component(
    provider: DynProvider,
    _to: Address,
    _data: Vec<u8>,
) -> Result<GasPriceEstimation, String> {
    let gas_price = provider.get_gas_price().await.map_err(|e| e.to_string())?;

    Ok(GasPriceEstimation {
        gas_estimate_for_l1: U256::ZERO,
        l2_base_fee: U256::from(gas_price),
        l1_data_fee: U256::ZERO,
    })
}
