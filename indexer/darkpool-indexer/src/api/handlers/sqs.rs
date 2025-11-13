//! Handler logic SQS messages polled by the darkpool indexer

use alloy::{primitives::TxHash, providers::Provider};
use aws_sdk_sqs::types::Message;
use darkpool_indexer_api::types::sqs::{MasterViewSeedMessage, RecoveryIdMessage, SqsMessage};
use renegade_constants::Scalar;

use crate::{
    api::handlers::error::HandlerError, indexer::Indexer, state_transitions::types::StateTransition,
};

// -----------------------------
// | Top-Level Message Handler |
// -----------------------------

/// Handle a message polled from SQS, parsing it into the API message type and
/// applying the appropriate handler logic
pub async fn handle_sqs_message(
    message: Message,
    indexer: &Indexer,
    sqs_queue_url: &str,
) -> Result<(), HandlerError> {
    if let Some(body) = message.body() {
        let message: SqsMessage = serde_json::from_str(body)?;
        match message {
            SqsMessage::RegisterMasterViewSeed(message) => {
                handle_master_view_seed_message(message, indexer).await?;
            },
            SqsMessage::RegisterRecoveryId(message) => {
                handle_recovery_id_message(message, indexer).await?;
            },
            SqsMessage::NullifierSpend(_) => {
                todo!()
            },
        }
    }

    if let Some(receipt_handle) = message.receipt_handle() {
        indexer
            .sqs_client
            .delete_message()
            .queue_url(sqs_queue_url)
            .receipt_handle(receipt_handle)
            .send()
            .await?;
    }

    Ok(())
}

// ------------
// | Handlers |
// ------------

// === Master View Seed Message Handler ===

/// Handle a SQS message representing the registration of a new master view seed
pub async fn handle_master_view_seed_message(
    message: MasterViewSeedMessage,
    indexer: &Indexer,
) -> Result<(), HandlerError> {
    let state_transition = StateTransition::RegisterMasterViewSeed(message);

    indexer.state_applicator.apply_state_transition(state_transition).await?;

    // TODO: Kick off backfill

    Ok(())
}

// === Recovery ID Message Handler ===

/// Handle an SQS message representing the registration of a new recovery ID
pub async fn handle_recovery_id_message(
    message: RecoveryIdMessage,
    indexer: &Indexer,
) -> Result<(), HandlerError> {
    let RecoveryIdMessage { recovery_id, tx_hash } = message;
    let state_transition =
        get_state_transition_for_recovery_id(recovery_id, tx_hash, indexer).await?;

    indexer.state_applicator.apply_state_transition(state_transition).await?;

    Ok(())
}

// -----------
// | Helpers |
// -----------

/// Get the state transition associated with the given recovery ID's
/// registration
async fn get_state_transition_for_recovery_id(
    _recovery_id: Scalar,
    tx_hash: TxHash,
    indexer: &Indexer,
) -> Result<StateTransition, HandlerError> {
    let registration_tx = indexer
        .ws_provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(HandlerError::rpc)?
        .ok_or(HandlerError::rpc(format!("Transaction receipt not found for tx {tx_hash:#x}")))?;

    let _block_number = registration_tx
        .block_number
        .ok_or(HandlerError::rpc("Block number not found in tx {tx_hash:#x} receipt"))?;

    todo!()
}
