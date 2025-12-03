//! Common utilities for integration tests

use alloy::primitives::U256;
use renegade_circuits::test_helpers::random_amount;

pub(crate) mod balance;
pub(crate) mod setup;
pub(crate) mod transactions;

/// Generate a random circuit-compatible amount as a U256.
///
/// The amount will be of size at most 2 ** AMOUNT_BITS
pub fn random_amount_u256() -> U256 {
    let amount_u128 = random_amount();
    U256::from(amount_u128)
}
