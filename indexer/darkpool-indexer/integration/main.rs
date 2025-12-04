//! Integration tests for the darkpool indexer

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]

use std::{path::PathBuf, sync::Arc};

use alloy::providers::ext::AnvilApi;
use clap::Parser;
use darkpool_indexer::{
    db::test_utils::cleanup_test_db,
    indexer::{
        run_message_queue_consumer, run_nullifier_spend_listener,
        run_recovery_id_registration_listener,
    },
};
use eyre::eyre;
use test_helpers::{integration_test_main, types::TestVerbosity};
use tokio::task::JoinSet;

use crate::{
    test_args::{TestArgs, TestContext},
    utils::setup::{build_test_indexer, register_test_master_view_seed, run_blocking_current},
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

integration_test_main!(CliArgs, TestArgs, setup, teardown);

// --------------------
// | Setup / Teardown |
// --------------------

/// Set up the test harness, populating the test context
fn setup(args: &TestArgs) {
    setup_tracing_subscriber(args.verbosity);

    run_blocking_current(async {
        // Build the test indexer instance, including a handle to a local PostgreSQL
        // instance
        let (indexer, postgres) =
            build_test_indexer(&args.anvil_ws_url, args.party0_signer.clone(), &args.deployments)
                .await?;

        // Snapshot the Anvil node's state to include all of the funding / other onchain
        // state setup downstream of `build_test_indexer`
        let anvil_snapshot_id = indexer.darkpool_client.provider().anvil_snapshot().await?;

        // Register the test account's master view seed into the indexer
        register_test_master_view_seed(&indexer, &args.party0_master_view_seed).await?;

        // Spawn the indexer tasks
        let mut indexer_tasks = JoinSet::new();
        indexer_tasks.spawn(run_message_queue_consumer(indexer.clone()));
        indexer_tasks.spawn(run_nullifier_spend_listener(indexer.clone()));
        indexer_tasks.spawn(run_recovery_id_registration_listener(indexer.clone()));

        let indexer_tasks = Arc::new(indexer_tasks);

        // Set the test context
        let test_context = TestContext { indexer, indexer_tasks, postgres, anvil_snapshot_id };

        args.test_context.set(test_context).map_err(|_| eyre!("Test context already set"))?;

        Ok::<_, eyre::Report>(())
    })
}

/// Tear down the test resources
fn teardown(mut test_args: TestArgs) {
    run_blocking_current(async {
        let context = test_args.test_context.take().unwrap();

        // Drain the DB client's pool before dropping the test database
        context.indexer.db_client.db_pool.close();

        // Clean up the test DB
        cleanup_test_db(&context.postgres).await?;

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
