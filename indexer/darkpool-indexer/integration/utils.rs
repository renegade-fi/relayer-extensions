//! Common utilities for integration tests

use darkpool_indexer::{
    chain_event_listener::ChainEventListener,
    darkpool_client::DarkpoolClient,
    db::test_utils::setup_test_db,
    indexer::Indexer,
    message_queue::{DynMessageQueue, mock_message_queue::MockMessageQueue},
    state_transitions::StateApplicator,
};
use eyre::Result;
use postgresql_embedded::PostgreSQL;

/// Construct a indexer instance for integration testing
pub async fn build_test_indexer() -> Result<(Indexer, PostgreSQL)> {
    // Set up a test DB client & state applicator
    let (db_client, postgres) = setup_test_db().await?;
    let state_applicator = StateApplicator::new(db_client.clone());

    // Set up the mock message queue
    let message_queue = DynMessageQueue::new(MockMessageQueue::new());

    // Set up the darkpool client
    let darkpool_client: DarkpoolClient = build_test_darkpool_client();

    let chain_event_listener = ChainEventListener::new(
        darkpool_client.clone(),
        0, // nullifier_start_block
        0, // recovery_id_start_block
        message_queue.clone(),
    );

    let indexer = Indexer {
        db_client,
        state_applicator,
        message_queue,
        darkpool_client,
        chain_event_listener,
        http_auth_key: None,
    };

    Ok((indexer, postgres))
}

/// Construct a test darkpool client, targeting a local Anvil node w/ the
/// darkpool deployed
fn build_test_darkpool_client() -> DarkpoolClient {
    todo!()
}
