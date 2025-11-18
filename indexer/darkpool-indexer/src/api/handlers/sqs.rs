//! Handler logic SQS messages polled by the darkpool indexer

use aws_sdk_sqs::types::Message;
use darkpool_indexer_api::types::sqs::{
    CreatePublicIntentMessage, MasterViewSeedMessage, NullifierSpendMessage, RecoveryIdMessage,
    SqsMessage,
};

use crate::{
    indexer::{Indexer, error::IndexerError},
    state_transitions::StateTransition,
};

impl Indexer {
    // -----------------------------
    // | Top-Level Message Handler |
    // -----------------------------

    /// Handle a message polled from SQS, parsing it into the API message type
    /// and applying the appropriate handler logic
    pub async fn handle_sqs_message(&self, message: Message) -> Result<(), IndexerError> {
        if let Some(body) = message.body() {
            let message: SqsMessage = serde_json::from_str(body)?;
            match message {
                SqsMessage::RegisterMasterViewSeed(message) => {
                    self.handle_master_view_seed_message(message).await?;
                },
                SqsMessage::RegisterRecoveryId(message) => {
                    self.handle_recovery_id_message(message).await?;
                },
                SqsMessage::NullifierSpend(message) => {
                    self.handle_nullifier_spend_message(message).await?;
                },
                SqsMessage::CreatePublicIntent(message) => {
                    self.handle_create_public_intent_message(message).await?;
                },
            }
        }

        if let Some(receipt_handle) = message.receipt_handle() {
            self.sqs_client
                .delete_message()
                .queue_url(&self.sqs_queue_url)
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
    ) -> Result<(), IndexerError> {
        let account_id = message.account_id;
        let state_transition = StateTransition::RegisterMasterViewSeed(message);

        self.state_applicator.apply_state_transition(state_transition).await?;

        // Kick off a backfill for the user's state in the background, so that we can
        // delete the master view seed message from SQS immediately
        let self_clone = self.clone();
        tokio::spawn(async move { self_clone.backfill_user_state(account_id).await });

        Ok(())
    }

    // === Recovery ID Message Handler ===

    /// Handle an SQS message representing the registration of a new recovery ID
    pub async fn handle_recovery_id_message(
        &self,
        message: RecoveryIdMessage,
    ) -> Result<(), IndexerError> {
        let RecoveryIdMessage { recovery_id, tx_hash } = message;
        let state_transition =
            self.get_state_transition_for_recovery_id(recovery_id, tx_hash).await?;

        if let Some(state_transition) = state_transition {
            self.state_applicator.apply_state_transition(state_transition).await?;
        }

        Ok(())
    }

    // === Nullifier Spend Message Handler ===

    /// Handle an SQS message representing the spending of a state object's
    /// nullifier onchain
    pub async fn handle_nullifier_spend_message(
        &self,
        message: NullifierSpendMessage,
    ) -> Result<(), IndexerError> {
        let NullifierSpendMessage { nullifier, tx_hash } = message;
        let state_transition = self.get_state_transition_for_nullifier(nullifier, tx_hash).await?;

        self.state_applicator.apply_state_transition(state_transition).await?;

        Ok(())
    }

    // === Public Intent Creation Message Handler ===

    /// Handle an SQS message representing the creation of a new public intent
    pub async fn handle_create_public_intent_message(
        &self,
        message: CreatePublicIntentMessage,
    ) -> Result<(), IndexerError> {
        let CreatePublicIntentMessage { intent_hash, owner_address, tx_hash } = message;
        let state_transition = self
            .get_state_transition_for_public_intent_creation(intent_hash, owner_address, tx_hash)
            .await?;

        self.state_applicator.apply_state_transition(state_transition).await?;

        Ok(())
    }
}
