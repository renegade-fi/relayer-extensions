//! The darkpool indexer, responsible for maintaining views of committed user
//! state

use std::sync::Arc;

use clap::Parser;
use darkpool_indexer::{
    api::http::routes::http_routes,
    cli::Cli,
    indexer::{
        Indexer, error::IndexerError, run_message_queue_consumer, run_nullifier_spend_listener,
        run_public_intent_cancellation_listener, run_public_intent_update_listener,
        run_recovery_id_registration_listener,
    },
};
use tokio::task::JoinSet;
use tracing::{error, warn};

// --------
// | Main |
// --------

#[tokio::main]
async fn main() -> Result<(), IndexerError> {
    let cli = Cli::parse();

    let indexer = Indexer::build_from_cli(&cli).await?;

    let mut tasks = JoinSet::new();

    // Spawn message queue consumer
    tasks.spawn(run_message_queue_consumer(indexer.clone()));

    // Spawn onchain event listeners
    tasks.spawn(run_nullifier_spend_listener(indexer.clone()));
    tasks.spawn(run_recovery_id_registration_listener(indexer.clone()));
    tasks.spawn(run_public_intent_update_listener(indexer.clone()));
    tasks.spawn(run_public_intent_cancellation_listener(indexer.clone()));

    // Spawn HTTP server
    tasks.spawn(run_http_server(indexer.clone(), cli.http_port));

    match tasks.join_next().await.expect("No tasks spawned") {
        Err(e) => error!("Error joining indexer task: {e}"),
        Ok(Ok(())) => warn!("Indexer task exited"),
        Ok(Err(e)) => error!("Indexer task error: {e}"),
    }

    Ok(())
}

/// Run the HTTP API server
async fn run_http_server(indexer: Arc<Indexer>, port: u16) -> Result<(), IndexerError> {
    warp::serve(http_routes(indexer)).run(([0, 0, 0, 0], port)).await;
    Ok(())
}
