//! Handler logic SQS messages polled by the darkpool indexer

use alloy::{providers::Provider, rpc::types::TransactionReceipt};
use aws_sdk_sqs::types::Message;
use darkpool_indexer_api::types::sqs::{MasterViewSeedMessage, NullifierSpendMessage, SqsMessage};
use renegade_constants::Scalar;

use crate::{
    api::handlers::error::HandlerError,
    indexer::Indexer,
    types::{MasterViewSeed, NullifierSpendData, StateObjectType},
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
            SqsMessage::NullifierSpend(message) => {
                handle_nullifier_spend_message(message, indexer).await?;
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
    let MasterViewSeedMessage { account_id, owner_address, seed } = message;

    let master_view_seed = MasterViewSeed::new(account_id, owner_address, seed);
    indexer.db.index_master_view_seed(master_view_seed).await?;

    // TODO: Kick off backfill

    Ok(())
}

// === Nullifier Spend Message Handler ===

/// Handle a SQS message representing the spending of a state object's nullifier
pub async fn handle_nullifier_spend_message(
    message: NullifierSpendMessage,
    indexer: &Indexer,
) -> Result<(), HandlerError> {
    let nullifier_spend_data = fetch_nullifier_spend_data(&message, indexer).await?;

    indexer.db.index_nullifier_spend(nullifier_spend_data).await?;

    Ok(())
}

/// Fetch the data necessary to index a nullifier spend
async fn fetch_nullifier_spend_data(
    nullifier_spend: &NullifierSpendMessage,
    indexer: &Indexer,
) -> Result<NullifierSpendData, HandlerError> {
    let tx_hash = nullifier_spend.tx_hash;
    let spend_tx = indexer
        .ws_provider
        .get_transaction_receipt(tx_hash)
        .await
        .map_err(HandlerError::rpc)?
        .ok_or(HandlerError::rpc(format!("Transaction receipt not found for tx {tx_hash:#x}")))?;

    let block_number = spend_tx
        .block_number
        .ok_or(HandlerError::rpc(format!("Block number not found in tx {tx_hash:#x} receipt")))?;

    let state_object_type =
        get_updated_state_object_type(nullifier_spend.nullifier, &spend_tx, indexer).await?;

    let (updated_public_shares, updated_shares_index) =
        get_updated_public_shares(nullifier_spend.nullifier, &spend_tx, indexer).await?;

    Ok(NullifierSpendData {
        nullifier: nullifier_spend.nullifier,
        block_number,
        state_object_type,
        updated_public_shares,
        updated_shares_index,
    })
}

/// Get the type of the state object associated with the spent nullifier
async fn get_updated_state_object_type(
    _nullifier: Scalar,
    _tx: &TransactionReceipt,
    _indexer: &Indexer,
) -> Result<StateObjectType, HandlerError> {
    todo!()
}

/// Get the updated public shares associated with a nullifier spend, along with
/// their start index within the secret-sharing of the state object
async fn get_updated_public_shares(
    _nullifier: Scalar,
    _tx: &TransactionReceipt,
    _indexer: &Indexer,
) -> Result<(Vec<Scalar>, usize), HandlerError> {
    todo!()
}
