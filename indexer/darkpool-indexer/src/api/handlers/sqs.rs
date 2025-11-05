//! Handler logic SQS messages polled by the darkpool indexer

use std::str::FromStr;

use alloy_primitives::Address;
use aws_sdk_sqs::types::Message;
use darkpool_indexer_api::types::sqs::{MasterViewSeedMessage, NullifierSpendMessage, SqsMessage};
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::warn;

use crate::{
    api::handlers::error::HandlerError,
    crypto_mocks::{
        csprng::PoseidonCSPRNG,
        encryption_stream::sample_encryption_seed,
        identifier_stream::{sample_identifier_seed, sample_nullifier},
    },
    db::{client::DbConn, error::DbError},
    indexer::Indexer,
    types::{ExpectedStateObject, GenericStateObject, MasterViewSeed, StateObjectType},
};

// -------------
// | Constants |
// -------------

/// The index of the identifier stream seed for an account's first state object
const FIRST_OBJECT_IDX: usize = 0;

/// A state object's first version
const OBJECT_FIRST_VERSION: usize = 0;

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

/// Handle a SQS message representing the registration of a new master view seed
pub async fn handle_master_view_seed_message(
    message: MasterViewSeedMessage,
    indexer: &Indexer,
) -> Result<(), HandlerError> {
    let MasterViewSeedMessage { account_id, owner_address, seed } = message;

    let owner_address_alloy = Address::from_str(&owner_address).map_err(HandlerError::parse)?;

    // Sample the identifier stream seed, first nullifier, and encryption stream
    // seed for the account's first state object
    let first_object_identifier_seed = sample_identifier_seed(seed, FIRST_OBJECT_IDX);
    let first_nullifier = sample_nullifier(first_object_identifier_seed, OBJECT_FIRST_VERSION);
    let first_object_encryption_seed = sample_encryption_seed(seed, FIRST_OBJECT_IDX);

    // Create the master view seed and expected state object
    let master_view_seed = MasterViewSeed { account_id, owner_address: owner_address_alloy, seed };

    let expected_state_object = ExpectedStateObject {
        nullifier: first_nullifier,
        account_id,
        owner_address: owner_address_alloy,
        identifier_seed: first_object_identifier_seed,
        encryption_seed: first_object_encryption_seed,
    };

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

/// Handle a SQS message representing the spending of a state object's nullifier
pub async fn handle_nullifier_spend_message(
    message: NullifierSpendMessage,
    indexer: &Indexer,
) -> Result<(), HandlerError> {
    let mut conn = indexer.db.get_db_conn().await?;
    conn.transaction(|conn| {
        async move {
            // Extract the nullifier and block number from the message before we move it
            let nullifier = message.nullifier;
            let block_number = message.block_number;

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
                handle_first_object_nullifier_spend(message, expected_state_object, indexer, conn)
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

/// Handle the spending of a state object's first nullifier
async fn handle_first_object_nullifier_spend(
    message: NullifierSpendMessage,
    expected_state_object: ExpectedStateObject,
    indexer: &Indexer,
    conn: &mut DbConn<'_>,
) -> Result<(), DbError> {
    let stream_cipher = PoseidonCSPRNG::new(expected_state_object.encryption_seed);
    let private_shares: Vec<Scalar> = stream_cipher.take(message.public_shares.len()).collect();

    let generic_state_object = GenericStateObject::new(
        expected_state_object.identifier_seed,
        expected_state_object.account_id,
        message.object_type.into(),
        expected_state_object.nullifier,
        expected_state_object.encryption_seed,
        expected_state_object.owner_address,
        message.public_shares,
        private_shares,
    );

    indexer.db.create_generic_state_object(generic_state_object.clone(), conn).await?;

    // TODO: Create the appropriate intent/balance state object
    match generic_state_object.object_type {
        StateObjectType::Intent => todo!(),
        StateObjectType::Balance => todo!(),
    }
}
