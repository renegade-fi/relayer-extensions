//! Integration tests for the darkpool indexer

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]

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

integration_test_main!(CliArgs, TestArgs, setup);

// --------------------
// | Setup / Teardown |
// --------------------

/// Set up the test harness, populating the test context
fn setup(args: &TestArgs) {
    setup_tracing_subscriber(args.verbosity);

    run_blocking_current(async {
        // Set up the darkpool client
        let darkpool_client = build_test_darkpool_client(
            &args.anvil_ws_url,
            args.party0_signer.clone(),
            &args.deployments,
        )
        .await?;

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
