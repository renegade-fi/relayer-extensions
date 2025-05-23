//! Chain specific gas oracle contract methods
use alloy_primitives::U256;

#[cfg(feature = "arbitrum")]
mod arbitrum;
#[cfg(feature = "base")]
mod base;

#[cfg(feature = "arbitrum")]
pub use arbitrum::estimate_l1_gas_component;
#[cfg(feature = "base")]
pub use base::estimate_l1_gas_component;

/// Result of the gas price estimation
pub struct GasPriceEstimation {
    /// The L1 gas estimate in L2 gas units
    pub gas_estimate_for_l1: U256,
    /// The L2 base fee in wei
    pub l2_base_fee: U256,
    /// The L1 base fee estimate (per byte) in wei
    pub l1_data_fee: U256,
}
