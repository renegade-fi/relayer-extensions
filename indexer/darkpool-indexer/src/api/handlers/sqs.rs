//! Handler logic SQS messages polled by the darkpool indexer

use aws_sdk_sqs::types::Message;
use darkpool_indexer_api::types::sqs::{MasterViewSeedMessage, RecoveryIdMessage, SqsMessage};

use crate::{
    api::handlers::error::HandlerError, indexer::Indexer, state_transitions::types::StateTransition,
};

impl Indexer {
    // -----------------------------
    // | Top-Level Message Handler |
    // -----------------------------

    /// Handle a message polled from SQS, parsing it into the API message type
    /// and applying the appropriate handler logic
    pub async fn handle_sqs_message(
        &self,
        message: Message,
        sqs_queue_url: &str,
    ) -> Result<(), HandlerError> {
        if let Some(body) = message.body() {
            let message: SqsMessage = serde_json::from_str(body)?;
            match message {
                SqsMessage::RegisterMasterViewSeed(message) => {
                    self.handle_master_view_seed_message(message).await?;
                },
                SqsMessage::RegisterRecoveryId(message) => {
                    self.handle_recovery_id_message(message).await?;
                },
                SqsMessage::NullifierSpend(_) => {
                    todo!()
                },
            }
        }

        if let Some(receipt_handle) = message.receipt_handle() {
            self.sqs_client
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

    /// Handle a SQS message representing the registration of a new master view
    /// seed
    pub async fn handle_master_view_seed_message(
        &self,
        message: MasterViewSeedMessage,
    ) -> Result<(), HandlerError> {
        let state_transition = StateTransition::RegisterMasterViewSeed(message);

        self.state_applicator.apply_state_transition(state_transition).await?;

        // TODO: Kick off backfill

        Ok(())
    }

    // === Recovery ID Message Handler ===

    /// Handle an SQS message representing the registration of a new recovery ID
    pub async fn handle_recovery_id_message(
        &self,
        message: RecoveryIdMessage,
    ) -> Result<(), HandlerError> {
        let RecoveryIdMessage { recovery_id, tx_hash } = message;
        let state_transition =
            self.darkpool_client.get_state_transition_for_recovery_id(recovery_id, tx_hash).await?;

        self.state_applicator.apply_state_transition(state_transition).await?;

        Ok(())
    }
}
