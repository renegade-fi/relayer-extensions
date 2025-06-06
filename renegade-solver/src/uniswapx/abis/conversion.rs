//! Conversion helpers for UniswapX ABI types

use alloy::primitives::U256;

use crate::error::{SolverError, SolverResult};

// ----------------------
// | Conversion Helpers |
// ----------------------

/// Parse a u128 from a U256
pub(crate) fn u256_to_u128(u256: U256) -> SolverResult<u128> {
    u256.try_into().map_err(|_| SolverError::InvalidU256(u256))
}
