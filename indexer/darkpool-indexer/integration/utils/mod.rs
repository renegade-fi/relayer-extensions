//! Common utilities for integration tests

use alloy::primitives::U256;
use renegade_circuits::test_helpers::random_amount;

pub(crate) mod balance;
pub(crate) mod merkle;
pub(crate) mod setup;
pub(crate) mod transactions;

/// Generate a random circuit-compatible amount as a U256.
///
/// The amount will be of size at most 2 ** AMOUNT_BITS
pub fn random_amount_u256() -> U256 {
    let amount_u128 = random_amount();
    U256::from(amount_u128)
}

/// Macro to create an async integration test which reverts the Anvil state
/// before invoking the test function
#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! indexer_integration_test {
    ($test_fn:ident) => {
        inventory::submit!(crate::TestWrapper(test_helpers::types::IntegrationTest {
            name: std::concat! {std::module_path!(), "::", stringify!($test_fn)},
            test_fn: test_helpers::types::IntegrationTestFn::AsynchronousFn(move |args| {
                std::boxed::Box::pin(async move {
                    args.revert_anvil_snapshot().await?;
                    $test_fn(args).await
                })
            }),
        }));
    };
}
