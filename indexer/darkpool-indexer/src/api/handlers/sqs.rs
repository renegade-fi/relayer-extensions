//! Handler logic SQS messages polled by the darkpool indexer

use alloy::{providers::Provider, rpc::types::TransactionReceipt};
use aws_sdk_sqs::types::Message;
use darkpool_indexer_api::types::sqs::{MasterViewSeedMessage, NullifierSpendMessage, SqsMessage};
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::warn;

use crate::{
    api::handlers::error::HandlerError,
    db::{client::DbConn, error::DbError},
    indexer::Indexer,
    types::{ExpectedStateObject, GenericStateObject, MasterViewSeed, StateObjectType},
};

// -------------
// | Constants |
// -------------

/// The index of an account's first state object
const FIRST_OBJECT_IDX: u64 = 0;

// ---------
// | Types |
// ---------

/// The data associated with a nullifier spend that is necessary for proper
/// indexing
struct NullifierSpendData {
    /// The nullifier that was spent
    nullifier: Scalar,
    /// The block number in which the nullifier was spent
    block_number: u64,
    /// The type of the state object that was updated
    state_object_type: StateObjectType,
    /// The updated public shares of the state object
    updated_public_shares: Vec<Scalar>,
    /// The start index of the updated public shares within the secret-sharing
    /// of the state object
    updated_shares_index: usize,
}

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

    let first_object_recovery_stream_seed =
        master_view_seed.recovery_seed_csprng.get_ith(FIRST_OBJECT_IDX);

    let first_object_share_stream_seed =
        master_view_seed.share_seed_csprng.get_ith(FIRST_OBJECT_IDX);

    let expected_state_object = ExpectedStateObject::new(
        account_id,
        owner_address,
        first_object_recovery_stream_seed,
        first_object_share_stream_seed,
    );

    // Insert the master view seed and expected state object into the database
    let mut conn = indexer.db.get_db_conn().await?;
    conn.transaction(|conn| {
        async move {
            indexer.db.insert_master_view_seed(master_view_seed, conn).await?;
            indexer.db.insert_expected_state_object(expected_state_object, conn).await
        }
        .scope_boxed()
    })
    .await?;

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

    let mut conn = indexer.db.get_db_conn().await?;
    conn.transaction(|conn| {
        async move {
            // Extract the nullifier and block number from the data before we move it
            let nullifier = nullifier_spend_data.nullifier;
            let block_number = nullifier_spend_data.block_number;

            // Check if the nullifier has already been processed
            let nullifier_processed = indexer.db.check_nullifier_processed(nullifier, conn).await?;
            if nullifier_processed {
                warn!("Nullifier {} has already been processed", nullifier);
                return Ok(());
            }

            // Check if this is the nullifier for an expected state object
            let maybe_expected_state_object =
                indexer.db.get_expected_state_object(nullifier, conn).await?;

            if let Some(expected_state_object) = maybe_expected_state_object {
                handle_first_object_nullifier_spend(
                    nullifier_spend_data,
                    expected_state_object,
                    indexer,
                    conn,
                )
                .await?;
            } else {
                // TODO: Handle nullifier spend messages for existing state
                // objects
            }

            indexer.db.mark_nullifier_processed(nullifier, block_number, conn).await
        }
        .scope_boxed()
    })
    .await?;

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
        .ok_or(HandlerError::rpc("Block number not found in tx {tx_hash:#x} receipt"))?;

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

/// Handle the spending of a state object's first nullifier
async fn handle_first_object_nullifier_spend(
    nullifier_spend_data: NullifierSpendData,
    expected_state_object: ExpectedStateObject,
    indexer: &Indexer,
    conn: &mut DbConn<'_>,
) -> Result<(), DbError> {
    let NullifierSpendData { updated_public_shares, state_object_type, .. } = nullifier_spend_data;

    let private_shares: Vec<Scalar> =
        expected_state_object.share_stream.clone().take(updated_public_shares.len()).collect();

    let generic_state_object = GenericStateObject::new(
        expected_state_object.recovery_stream.seed,
        expected_state_object.account_id,
        state_object_type,
        expected_state_object.nullifier,
        expected_state_object.share_stream.seed,
        expected_state_object.owner_address,
        updated_public_shares,
        private_shares,
    );

    indexer.db.create_generic_state_object(generic_state_object.clone(), conn).await?;

    // TODO: Create the appropriate intent/balance state object
    match generic_state_object.object_type {
        StateObjectType::Intent => todo!(),
        StateObjectType::Balance => todo!(),
    }

    // TODO: Delete expected state object record
}
