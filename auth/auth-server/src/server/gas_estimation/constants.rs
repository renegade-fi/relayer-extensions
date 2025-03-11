//! Constants used in gas estimation

use std::time::Duration;

use alloy_primitives::hex;
use ethers::types::{Address, H160};

/// A pessimistic overestimate of the gas cost of L2 execution for an external
/// match, rounded up to the nearest million.
pub const ESTIMATED_L2_GAS: u64 = 4_000_000; // 4m

/// The approximate size in bytes of the calldata for an external match,
/// obtained empirically
pub const ESTIMATED_CALLDATA_SIZE_BYTES: usize = 8_000;

/// The address of the `NodeInterface` precompile
pub const NODE_INTERFACE_ADDRESS: Address = H160(hex!("00000000000000000000000000000000000000c8"));

/// The interval at which to sample the gas cost of an external match
pub const GAS_COST_SAMPLING_INTERVAL: Duration = Duration::from_secs(10);
