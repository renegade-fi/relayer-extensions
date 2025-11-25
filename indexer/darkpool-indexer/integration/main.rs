//! Integration tests for the darkpool indexer

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]

use std::path::PathBuf;

use alloy::{primitives::U256, providers::ext::AnvilApi};
use clap::Parser;
use darkpool_indexer::indexer::Indexer;
use postgresql_embedded::PostgreSQL;
use renegade_test_helpers::{integration_test_main, types::TestVerbosity};

use crate::utils::{build_test_indexer, run_blocking};

mod utils;

// -------------
// | Constants |
// -------------

/// The default anvil private key
const DEFAULT_ANVIL_PKEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

// ---------------------
// | Test Harness Args |
// ---------------------

/// The arguments used to run the integration tests
#[derive(Parser, Clone)]
struct CliArgs {
    /// The WS RPC URL of a local Anvil node w/ the darkpool contracts deployed
    #[clap(long, default_value = "ws://localhost:8545")]
    anvil_ws_url: String,
    /// The private key of the test wallet, assumed to be funded w/ ETH
    #[clap(long, default_value = DEFAULT_ANVIL_PKEY)]
    pkey: String,
    /// The path to the contract deployments file
    #[clap(long)]
    deployments: PathBuf,
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
    /// The ID of the Anvil snapshot from which to run each test
    anvil_snapshot_id: U256,
}

impl From<CliArgs> for TestArgs {
    fn from(value: CliArgs) -> Self {
        let (indexer, postgres, anvil_snapshot_id) = run_blocking(async {
            let (indexer, postgres) =
                build_test_indexer(&value.anvil_ws_url, &value.pkey, &value.deployments).await?;

            let anvil_snapshot_id = indexer.darkpool_client.provider().anvil_snapshot().await?;

            Ok::<_, eyre::Report>((indexer, postgres, anvil_snapshot_id))
        });

        Self { indexer, postgres, anvil_snapshot_id }
    }
}

integration_test_main!(CliArgs, TestArgs);
