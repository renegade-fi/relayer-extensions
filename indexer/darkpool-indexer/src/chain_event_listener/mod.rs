//! An onchain event listener, subscribes to relevant darkpool events and
//! forwards them to the indexer

use darkpool_indexer_api::types::message_queue::{
    CancelPublicIntentMessage, Message, NullifierSpendMessage, RecoveryIdMessage,
    UpdatePublicIntentMessage,
};
use futures_util::StreamExt;
use renegade_crypto::fields::u256_to_scalar;
use tracing::info;

use crate::{
    chain_event_listener::error::ChainEventListenerError,
    darkpool_client::DarkpoolClient,
    message_queue::{DynMessageQueue, MessageQueue},
};

pub mod error;

/// The chain event listener
#[derive(Clone)]
pub struct ChainEventListener {
    /// The darkpool client, expected to be configured with a websocket provider
    pub darkpool_client: DarkpoolClient,
    /// The block number from which to start listening for nullifier spend
    /// events
    pub nullifier_start_block: u64,
    /// The block number from which to start listening for recovery ID
    /// registration events
    pub recovery_id_start_block: u64,
    /// The block number from which to start listening for public intent
    /// update events
    pub public_intent_update_start_block: u64,
    /// The block number from which to start listening for public intent
    /// cancellation events
    pub public_intent_cancellation_start_block: u64,
    /// The message queue
    pub message_queue: DynMessageQueue<Message>,
}

impl ChainEventListener {
    /// Create a new chain event listener
    pub fn new(
        darkpool_client: DarkpoolClient,
        nullifier_start_block: u64,
        recovery_id_start_block: u64,
        public_intent_update_start_block: u64,
        public_intent_cancellation_start_block: u64,
        message_queue: DynMessageQueue<Message>,
    ) -> Self {
        Self {
            darkpool_client,
            nullifier_start_block,
            recovery_id_start_block,
            public_intent_update_start_block,
            public_intent_cancellation_start_block,
            message_queue,
        }
    }

    /// Watch for nullifier spend events and forward them to the message queue
    pub async fn watch_nullifiers(&self) -> Result<(), ChainEventListenerError> {
        let filter = self
            .darkpool_client
            .darkpool
            .NullifierSpent_filter()
            .from_block(self.nullifier_start_block);

        info!("Listening for nullifier spend events from block {}", self.nullifier_start_block);
        let mut stream = filter.subscribe().await?.into_stream();

        while let Some(Ok((event, log))) = stream.next().await {
            let nullifier = u256_to_scalar(event.nullifier);
            let tx_hash = log.transaction_hash.ok_or(ChainEventListenerError::rpc(format!(
                "no tx hash for nullifier {nullifier} spend event"
            )))?;

            let message = Message::NullifierSpend(NullifierSpendMessage { nullifier, tx_hash });

            let nullifier_str = nullifier.to_string();

            self.message_queue.send_message(message, nullifier_str.clone(), nullifier_str).await?;
        }

        Ok(())
    }

    /// Watch for recovery ID registration events and forward them to the
    /// message queue
    pub async fn watch_recovery_ids(&self) -> Result<(), ChainEventListenerError> {
        let filter = self
            .darkpool_client
            .darkpool
            .RecoveryIdRegistered_filter()
            .from_block(self.recovery_id_start_block);

        info!(
            "Listening for recovery ID registration events from block {}",
            self.recovery_id_start_block
        );
        let mut stream = filter.subscribe().await?.into_stream();

        while let Some(Ok((event, log))) = stream.next().await {
            let recovery_id = u256_to_scalar(event.recoveryId);
            let tx_hash = log.transaction_hash.ok_or(ChainEventListenerError::rpc(format!(
                "no tx hash for recovery ID {recovery_id} registration event"
            )))?;

            let message = Message::RegisterRecoveryId(RecoveryIdMessage { recovery_id, tx_hash });

            let recovery_id_str = recovery_id.to_string();

            self.message_queue
                .send_message(message, recovery_id_str.clone(), recovery_id_str)
                .await?;
        }

        Ok(())
    }

    /// Watch for public intent update events and forward them to the message
    /// queue
    pub async fn watch_public_intent_updates(&self) -> Result<(), ChainEventListenerError> {
        let filter = self
            .darkpool_client
            .darkpool
            .PublicIntentUpdated_filter()
            .from_block(self.public_intent_update_start_block);

        info!(
            "Listening for public intent update events from block {}",
            self.public_intent_update_start_block
        );
        let mut stream = filter.subscribe().await?.into_stream();

        while let Some(Ok((event, log))) = stream.next().await {
            let intent_hash = event.intentHash;
            let tx_hash = log.transaction_hash.ok_or(ChainEventListenerError::rpc(format!(
                "no tx hash for public intent {intent_hash} update event"
            )))?;

            let message =
                Message::UpdatePublicIntent(UpdatePublicIntentMessage { intent_hash, tx_hash });

            let intent_hash_str = intent_hash.to_string();
            let tx_hash_str = tx_hash.to_string();

            self.message_queue.send_message(message, tx_hash_str, intent_hash_str).await?;
        }

        Ok(())
    }

    /// Watch for public intent cancellation events and forward them to the
    /// message queue
    pub async fn watch_public_intent_cancellations(&self) -> Result<(), ChainEventListenerError> {
        let filter = self
            .darkpool_client
            .darkpool
            .PublicIntentCancelled_filter()
            .from_block(self.public_intent_cancellation_start_block);

        info!(
            "Listening for public intent cancellation events from block {}",
            self.public_intent_cancellation_start_block
        );
        let mut stream = filter.subscribe().await?.into_stream();

        while let Some(Ok((event, log))) = stream.next().await {
            let intent_hash = event.intentHash;
            let tx_hash = log.transaction_hash.ok_or(ChainEventListenerError::rpc(format!(
                "no tx hash for public intent {intent_hash} cancellation event"
            )))?;

            let message =
                Message::CancelPublicIntent(CancelPublicIntentMessage { intent_hash, tx_hash });

            let intent_hash_str = intent_hash.to_string();
            let tx_hash_str = tx_hash.to_string();

            self.message_queue.send_message(message, tx_hash_str, intent_hash_str).await?;
        }

        Ok(())
    }
}
