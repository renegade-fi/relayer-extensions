//! Integration tests for the darkpool indexer

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]

use clap::Parser;
use darkpool_indexer::indexer::Indexer;
use postgresql_embedded::PostgreSQL;
use renegade_test_helpers::{integration_test_main, types::TestVerbosity};
use tokio::runtime::Runtime;

use crate::utils::build_test_indexer;

mod utils;

// ---------------------
// | Test Harness Args |
// ---------------------

/// The arguments used to run the integration tests
#[derive(Parser, Clone)]
struct CliArgs {
    /// The test to run
    #[clap(long, short)]
    test: Option<String>,
    /// The verbosity with which to run the tests
    #[clap(long, short, default_value = "default")]
    verbosity: TestVerbosity,
}

#[derive(Clone)]
struct TestArgs {
    /// The indexer instance to test
    indexer: Indexer,
    /// The local PostgreSQL instance to use for testing
    postgres: PostgreSQL,
}

impl From<CliArgs> for TestArgs {
    fn from(value: CliArgs) -> Self {
        let (indexer, postgres) = Runtime::new().unwrap().block_on(build_test_indexer()).unwrap();

        Self { indexer, postgres }
    }
}

integration_test_main!(CliArgs, TestArgs);
