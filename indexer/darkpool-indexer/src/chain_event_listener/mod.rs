//! An onchain event listener, subscribes to relevant darkpool events and
//! forwards them to the indexer

use aws_sdk_sqs::Client as SqsClient;
use darkpool_indexer_api::types::sqs::{NullifierSpendMessage, RecoveryIdMessage};
use futures_util::StreamExt;
use renegade_crypto::fields::u256_to_scalar;
use tracing::info;

use crate::{
    chain_event_listener::error::ChainEventListenerError, darkpool_client::DarkpoolClient,
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
    /// The AWS SQS client
    pub sqs_client: SqsClient,
}

impl ChainEventListener {
    /// Create a new chain event listener
    pub fn new(
        darkpool_client: DarkpoolClient,
        nullifier_start_block: u64,
        recovery_id_start_block: u64,
        sqs_client: SqsClient,
    ) -> Self {
        Self { darkpool_client, nullifier_start_block, recovery_id_start_block, sqs_client }
    }

    /// Watch for nullifier spend events and forward them to the SQS queue
    pub async fn watch_nullifiers(
        &self,
        sqs_queue_url: String,
    ) -> Result<(), ChainEventListenerError> {
        let filter = self
            .darkpool_client
            .darkpool
            .NullifierSpent_filter()
            .from_block(self.nullifier_start_block);

        info!("Listening for nullifier spend events from block {}", self.nullifier_start_block);
        let mut stream = filter.subscribe().await?.into_stream();

        while let Some(Ok((event, log))) = stream.next().await {
            let nullifier = u256_to_scalar(&event.nullifier);
            let tx_hash = log
                .transaction_hash
                .ok_or(ChainEventListenerError::rpc("no tx hash for nullifier spend event"))?;

            let message = NullifierSpendMessage { nullifier, tx_hash };
            let message_body = serde_json::to_string(&message)?;

            let nullifier_str = nullifier.to_string();

            self.sqs_client
                .send_message()
                .queue_url(&sqs_queue_url)
                .message_deduplication_id(&nullifier_str)
                .message_group_id(&nullifier_str)
                .message_body(message_body)
                .send()
                .await?;
        }

        Ok(())
    }

    /// Watch for recovery ID registration events and forward them to the SQS
    /// queue
    pub async fn watch_recovery_ids(
        &self,
        sqs_queue_url: String,
    ) -> Result<(), ChainEventListenerError> {
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
            let recovery_id = u256_to_scalar(&event.recoveryId);
            let tx_hash = log.transaction_hash.ok_or(ChainEventListenerError::rpc(
                "no tx hash for recovery ID registration event",
            ))?;

            let message = RecoveryIdMessage { recovery_id, tx_hash };
            let message_body = serde_json::to_string(&message)?;

            let recovery_id_str = recovery_id.to_string();

            self.sqs_client
                .send_message()
                .queue_url(&sqs_queue_url)
                .message_deduplication_id(&recovery_id_str)
                .message_group_id(&recovery_id_str)
                .message_body(message_body)
                .send()
                .await?;
        }

        Ok(())
    }
}
