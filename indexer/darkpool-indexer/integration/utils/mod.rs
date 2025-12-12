//! Common utilities for integration tests

pub(crate) mod abis;
pub(crate) mod indexer_state;
pub(crate) mod merkle;
pub(crate) mod setup;
pub(crate) mod test_data;
pub(crate) mod transactions;

// -----------
// | Helpers |
// -----------

// ----------
// | Macros |
// ----------

/// Macro to create an async integration test which reverts the Anvil state
/// before invoking the test function
#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! indexer_integration_test {
    ($test_fn:ident) => {
        inventory::submit!(crate::TestWrapper(test_helpers::types::IntegrationTest {
            name: std::concat! {std::module_path!(), "::", stringify!($test_fn)},
            test_fn: test_helpers::types::IntegrationTestFn::AsynchronousFn(move |mut args| {
                std::boxed::Box::pin(async move {
                    args.revert_anvil_snapshot().await?;
                    args.inject_indexer_context().await?;

                    let res = $test_fn(args.clone()).await;

                    args.teardown_indexer_context().await?;

                    res
                })
            }),
        }));
    };
}
