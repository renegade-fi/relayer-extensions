//! The API for the funds manager
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod auth;
pub mod serialization;
mod types;
pub use types::*;

use alloy_primitives::{ruint::FromUintError, U256};

/// Helper to attempt to convert a U256 to a u128, returning a String error
/// if it fails
pub fn u256_try_into_u128(u256: U256) -> Result<u128, String> {
    u256.try_into().map_err(|e: FromUintError<u128>| e.to_string())
}

/// Helper to attempt to convert a U256 to a u64, returning a String error
/// if it fails
pub fn u256_try_into_u64(u256: U256) -> Result<u64, String> {
    u256.try_into().map_err(|e: FromUintError<u64>| e.to_string())
}
