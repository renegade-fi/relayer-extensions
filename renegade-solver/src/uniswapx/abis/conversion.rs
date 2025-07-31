//! Conversion helpers for UniswapX ABI types

use alloy::primitives::U256;

// ----------------------
// | Conversion Helpers |
// ----------------------

/// Parse a u128 from a U256
///
/// The function returns `Result<u128, U256>` where the `Err(U256)` variant
/// contains the original value that did not fit into 128 bits, to be handled
/// by the caller.
pub(crate) fn u256_to_u128(u256: U256) -> Result<u128, U256> {
    u256.try_into().map_err(|_| u256)
}
