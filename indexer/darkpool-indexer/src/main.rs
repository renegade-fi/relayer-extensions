//! The darkpool indexer, responsible for maintaining views of committed user
//! state

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]
#![feature(let_chains)]
#![feature(trait_alias)]

use std::sync::Arc;

use clap::Parser;
use tokio::task::JoinSet;
use tracing::{error, warn};

use crate::{
    api::http::routes::http_routes,
    cli::Cli,
    indexer::{Indexer, error::IndexerError},
    message_queue::MessageQueue,
};

mod api;
mod chain_event_listener;
mod cli;
mod crypto_mocks;
mod darkpool_client;
mod db;
mod indexer;
mod message_queue;
mod state_transitions;
mod types;

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
    // TODO: Run public intent event listeners

    // Spawn HTTP server
    tasks.spawn(run_http_server(indexer.clone(), cli.http_port));

    match tasks.join_next().await.expect("No tasks spawned") {
        Err(e) => error!("Error joining indexer task: {e}"),
        Ok(Ok(())) => warn!("Indexer task exited"),
        Ok(Err(e)) => error!("Indexer task error: {e}"),
    }

    Ok(())
}

/// Run the message queue consumer, polling for new messages from the
/// queue and handling them
async fn run_message_queue_consumer(indexer: Arc<Indexer>) -> Result<(), IndexerError> {
    loop {
        let message_groups = indexer.message_queue.poll_messages().await?;

        // Process message groups concurrently
        for messages in message_groups.into_values() {
            let indexer_clone = indexer.clone();
            tokio::spawn(async move {
                // Process messages within a message group sequentially
                for (message, deletion_id) in messages {
                    if let Err(e) = indexer_clone.handle_message(message, deletion_id).await {
                        error!("Error handling queue message: {e}")
                    }
                }
            });
        }
    }
}

/// Run the nullifier spend event listener, watching for nullifier spend events
/// and forwarding them to the message queue
async fn run_nullifier_spend_listener(indexer: Arc<Indexer>) -> Result<(), IndexerError> {
    indexer.chain_event_listener.watch_nullifiers().await?;
    Ok(())
}

/// Run the recovery ID registration event listener, watching for recovery ID
/// registration events and forwarding them to the message queue
async fn run_recovery_id_registration_listener(indexer: Arc<Indexer>) -> Result<(), IndexerError> {
    indexer.chain_event_listener.watch_recovery_ids().await?;
    Ok(())
}

/// Run the HTTP API server
async fn run_http_server(indexer: Arc<Indexer>, port: u16) -> Result<(), IndexerError> {
    warp::serve(http_routes(indexer)).run(([0, 0, 0, 0], port)).await;
    Ok(())
}
