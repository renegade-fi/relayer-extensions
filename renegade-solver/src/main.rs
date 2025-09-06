//! Entrypoint for the renegade solver

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(unsafe_code)]
#![deny(clippy::uninlined_format_args)]

use std::net::SocketAddr;

use clap::Parser;
use renegade_config::setup_token_remaps;
use serde_json::json;
use tracing::{info, info_span};
use warp::Filter;

use crate::{
    arrival_control::controller::ArrivalController,
    chain_events::listener::ChainEventsListener,
    chain_state_cache::{worker::ChainStateCacheWorker, ChainStateCache},
    cli::Cli,
    error::{handle_rejection, SolverError},
    flashblocks::{clock::FlashblockClock, FlashblocksListener},
    tx_driver::driver::TxDriver,
    tx_store::store::TxStore,
    uniswapx::{executor_client::ExecutorClient, UniswapXSolver},
};

mod arrival_control;
mod chain_events;
mod chain_state_cache;
mod cli;
mod error;
mod flashblocks;
mod tx_driver;
mod tx_store;
mod uniswapx;

/// Main entrypoint for the renegade solver server
#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    cli.configure_telemetry();
    setup_token_mapping(&cli).await.expect("Failed to setup token mapping");

    // Create the executor client
    let executor_client =
        ExecutorClient::new(&cli).await.expect("Failed to create executor client");

    // Create the base chain state cache
    let chain_state_cache = ChainStateCache::new(&cli);

    // Create the base chain state cache worker
    let chain_state_cache_worker =
        ChainStateCacheWorker::new(executor_client.provider(), chain_state_cache.clone());
    chain_state_cache_worker.start();

    // Create the TxStore
    let tx_store = TxStore::default();

    // Create the arrival controller
    let controller = ArrivalController::default();

    // Create flashblocks listener and start the subscription
    let flashblock_clock = FlashblockClock::new();
    let chain_listener = ChainEventsListener::new(tx_store.clone());

    let flashblocks_listener = FlashblocksListener::new(
        vec![Box::new(chain_listener), Box::new(flashblock_clock.clone())],
        &cli,
    );
    flashblocks_listener.start();

    let tx_driver = TxDriver::new(&controller, &executor_client, &tx_store);
    // Create the UniswapX solver and begin its polling loop
    let uniswapx = UniswapXSolver::new(
        cli.clone(),
        controller.clone(),
        executor_client.clone(),
        chain_state_cache.clone(),
        flashblock_clock,
        tx_driver.clone(),
        tx_store.clone(),
    )
    .await
    .expect("Failed to create UniswapX solver");
    uniswapx.spawn_polling_loop();

    // Create the endpoints
    info!("Starting renegade solver server on port {}", cli.port);
    let ping = ping_handler();

    // Add request tracing
    let routes = ping.with(with_tracing()).recover(handle_rejection);
    let listen_addr: SocketAddr = ([0, 0, 0, 0], cli.port).into();
    warp::serve(routes).run(listen_addr).await;
}

/// Creates the ping endpoint handler
fn ping_handler() -> impl Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("ping").and(warp::get()).map(|| warp::reply::json(&json!({ "message": "PONG" })))
}

// -----------
// | Helpers |
// -----------

/// Setup the token mapping
async fn setup_token_mapping(cli: &Cli) -> Result<(), SolverError> {
    let chain_id = cli.chain_id;
    tokio::task::spawn_blocking(move || {
        setup_token_remaps(None /* token remap file */, chain_id)
    })
    .await
    .unwrap()
    .expect("Failed to setup token mapping");
    Ok(())
}

/// Custom tracing filter that creates spans for requests at info level
/// with the renegade_solver::request target to work with our RUST_LOG
/// configuration
fn with_tracing() -> warp::trace::Trace<impl Fn(warp::trace::Info) -> tracing::Span + Clone> {
    warp::trace(|info| {
        let span = info_span!(
            target: "renegade_solver::request",
            "handle_request",
            method = %info.method(),
            path = %info.path(),
        );

        span
    })
}
