//! Constants used in gas estimation

use std::time::Duration;

use alloy_primitives::U256;

use crate::server::helpers::u64_to_u256;

/// A pessimistic overestimate of the gas cost of L2 execution for an external
/// match, rounded up to the nearest 100k.

/// The estimated L2 gas cost of submitting an external match settlement
pub const ESTIMATED_L2_GAS_U64: u64 = 3_600_000; // 3.6m

/// The estimated L2 gas cost as a U256
/// In the future, we can consider sampling execution gas costs from a recent
/// external match
pub const ESTIMATED_L2_GAS: U256 = u64_to_u256(ESTIMATED_L2_GAS_U64);

/// The approximate size in bytes of the calldata for an external match,
/// accounting for an expected compression ratio.
/// Concretely, our calldata is ~8kb, and we expect a compression ratio
/// of ~75%. Both of these values were obtained empirically.

// In the future, we can consider sampling calldata from a recent external match
pub const ESTIMATED_COMPRESSED_CALLDATA_SIZE_BYTES: usize = 6_000;

/// The interval at which to sample the gas cost of an external match
pub const GAS_COST_SAMPLING_INTERVAL: Duration = Duration::from_secs(10);
