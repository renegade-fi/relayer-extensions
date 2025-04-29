//! Constants used in gas estimation

use std::time::Duration;

use alloy::primitives::Address;
use alloy_primitives::{hex, FixedBytes};

/// A pessimistic overestimate of the gas cost of L2 execution for an external
/// match, rounded up to the nearest 100k.

// In the future, we can consider sampling execution gas costs from a recent
// external match
pub const ESTIMATED_L2_GAS: u64 = 3_600_000; // 3.6m

/// The approximate size in bytes of the calldata for an external match,
/// accounting for an expected compression ratio.
/// Concretely, our calldata is ~8kb, and we expect a compression ratio
/// of ~75%. Both of these values were obtained empirically.

// In the future, we can consider sampling calldata from a recent external match
pub const ESTIMATED_COMPRESSED_CALLDATA_SIZE_BYTES: usize = 6_000;

/// The address of the `NodeInterface` precompile
pub const NODE_INTERFACE_ADDRESS: Address =
    Address(FixedBytes(hex!("00000000000000000000000000000000000000c8")));

/// The interval at which to sample the gas cost of an external match
pub const GAS_COST_SAMPLING_INTERVAL: Duration = Duration::from_secs(10);
