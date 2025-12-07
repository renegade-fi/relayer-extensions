//! Integration tests for the darkpool indexer

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::unused_async)]

use std::path::PathBuf;

use alloy::providers::ext::AnvilApi;
use clap::Parser;
use eyre::eyre;
use test_helpers::{integration_test_main, types::TestVerbosity};

use crate::{
    test_args::{AnvilContext, TestArgs},
    utils::setup::{build_test_darkpool_client, run_blocking_current},
};

mod test_args;
mod tests;
pub(crate) mod utils;

// -------------
// | Constants |
// -------------

/// The first pre-allocated anvil private key
const FIRST_ANVIL_PKEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

/// The second pre-allocated anvil private key
const SECOND_ANVIL_PKEY: &str =
    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

// ---------------------
// | Test Harness Args |
// ---------------------

/// The arguments used to run the integration tests
#[derive(Parser, Clone)]
struct CliArgs {
    /// The WS RPC URL of a local Anvil node w/ the darkpool contracts deployed
    #[clap(long, default_value = "ws://localhost:8545")]
    anvil_ws_url: String,
    /// The private key of the first test wallet, assumed to be funded w/ ETH
    #[clap(long, default_value = FIRST_ANVIL_PKEY)]
    party0_pkey: String,
    /// The private key of the second test wallet, assumed to be funded w/ ETH
    #[clap(long, default_value = SECOND_ANVIL_PKEY)]
    party1_pkey: String,
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

integration_test_main!(CliArgs, TestArgs, setup);

// --------------------
// | Setup / Teardown |
// --------------------

/// Set up the test harness, populating the test context
fn setup(args: &TestArgs) {
    setup_tracing_subscriber(args.verbosity);

    run_blocking_current(async {
        // Set up the darkpool client
        let darkpool_client = build_test_darkpool_client(args).await?;

        // Snapshot the Anvil node's state to include all of the funding / other onchain
        // state setup downstream of `build_test_indexer`
        let anvil_snapshot_id = darkpool_client.provider().anvil_snapshot().await?;

        // Set the anvil context
        let anvil_context = AnvilContext { darkpool_client, anvil_snapshot_id };

        args.anvil_context.set(anvil_context).map_err(|_| eyre!("Anvil context already set"))?;

        Ok::<_, eyre::Report>(())
    })
}

// ----------------
// | Misc Helpers |
// ----------------

/// Set up the tracing subscriber for the integration tests
fn setup_tracing_subscriber(verbosity: TestVerbosity) {
    if matches!(verbosity, TestVerbosity::Quiet) {
        return;
    }

    tracing_subscriber::fmt().pretty().with_env_filter("darkpool_indexer=info").init();
}
