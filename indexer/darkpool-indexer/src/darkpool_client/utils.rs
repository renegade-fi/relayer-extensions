//! Common utilities for the darkpool client

use alloy::primitives::B256;
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u256;

// -------------
// | Constants |
// -------------

/// The length of a function selector in bytes
const NUM_BYTES_SELECTOR: usize = 4;

// -----------
// | Helpers |
// -----------

/// Convert a scalar to a B256
pub fn scalar_to_b256(scalar: Scalar) -> B256 {
    scalar_to_u256(&scalar).into()
}

/// Get the function selector from calldata
pub fn get_selector(calldata: &[u8]) -> [u8; NUM_BYTES_SELECTOR] {
    calldata[..NUM_BYTES_SELECTOR].try_into().unwrap()
}
