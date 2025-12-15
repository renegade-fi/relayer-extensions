//! Handler logic for messages polled from the message queue by the darkpool
//! indexer

use darkpool_indexer_api::types::message_queue::{
    CreatePublicIntentMessage, MasterViewSeedMessage, Message, NullifierSpendMessage,
    RecoveryIdMessage, UpdatePublicIntentMessage,
};
use tracing::info;

use crate::{
    indexer::{Indexer, error::IndexerError},
    message_queue::MessageQueue,
    state_transitions::StateTransition,
};

impl Indexer {
    // -----------------------------
    // | Top-Level Message Handler |
    // -----------------------------

    /// Handle a message polled from the message queue, parsing it into the API
    /// message type and applying the appropriate handler logic
    pub async fn handle_message(
        &self,
        message: Message,
        deletion_id: String,
    ) -> Result<(), IndexerError> {
        match message {
            Message::RegisterMasterViewSeed(message) => {
                self.handle_master_view_seed_message(message).await?;
            },
            Message::RegisterRecoveryId(message) => {
                self.handle_recovery_id_message(message).await?;
            },
            Message::NullifierSpend(message) => {
                self.handle_nullifier_spend_message(message).await?;
            },
            Message::CreatePublicIntent(message) => {
                self.handle_create_public_intent_message(message).await?;
            },
            Message::UpdatePublicIntent(message) => {
                self.handle_update_public_intent_message(message).await?;
            },
        }

        self.message_queue.delete_message(deletion_id).await?;

        Ok(())
    }

    // ------------
    // | Handlers |
    // ------------

    // === Master View Seed Message Handler ===

    /// Handle a message representing the registration of a new master view seed
    pub async fn handle_master_view_seed_message(
        &self,
        message: MasterViewSeedMessage,
    ) -> Result<(), IndexerError> {
        let account_id = message.account_id;
        let state_transition = StateTransition::RegisterMasterViewSeed(message);

        self.state_applicator.apply_state_transition(state_transition).await?;

        // Kick off a backfill for the user's state in the background, so that we can
        // delete the master view seed message from the queue immediately
        let self_clone = self.clone();
        tokio::spawn(async move { self_clone.backfill_user_state(account_id).await });

        Ok(())
    }

    // === Recovery ID Message Handler ===

    /// Handle a message representing the registration of a new recovery ID
    pub async fn handle_recovery_id_message(
        &self,
        message: RecoveryIdMessage,
    ) -> Result<(), IndexerError> {
        let RecoveryIdMessage { recovery_id, tx_hash } = message;
        let state_transition =
            self.get_state_transition_for_recovery_id(recovery_id, tx_hash).await?;

        if let Some(state_transition) = state_transition {
            info!(
                "Applying {} state transition for recovery ID {recovery_id}",
                state_transition.name()
            );

            self.state_applicator.apply_state_transition(state_transition).await?;
        }

        Ok(())
    }

    // === Nullifier Spend Message Handler ===

    /// Handle a message representing the spending of a state object's nullifier
    /// onchain
    pub async fn handle_nullifier_spend_message(
        &self,
        message: NullifierSpendMessage,
    ) -> Result<(), IndexerError> {
        info!("Handling nullifier spend message");

        let NullifierSpendMessage { nullifier, tx_hash } = message;
        let state_transition = self.get_state_transition_for_nullifier(nullifier, tx_hash).await?;

        self.state_applicator.apply_state_transition(state_transition).await?;

        Ok(())
    }

    // === Public Intent Creation Message Handler ===

    /// Handle a message representing the creation of a new public intent
    pub async fn handle_create_public_intent_message(
        &self,
        message: CreatePublicIntentMessage,
    ) -> Result<(), IndexerError> {
        let CreatePublicIntentMessage { intent_hash, tx_hash } = message;
        let state_transition =
            self.get_state_transition_for_public_intent_creation(intent_hash, tx_hash).await?;

        self.state_applicator.apply_state_transition(state_transition).await?;

        Ok(())
    }

    // === Public Intent Update Message Handler ===

    /// Handle a message representing the update of a public intent
    pub async fn handle_update_public_intent_message(
        &self,
        message: UpdatePublicIntentMessage,
    ) -> Result<(), IndexerError> {
        let UpdatePublicIntentMessage { intent_hash, version, tx_hash } = message;
        let state_transition = self
            .get_state_transition_for_public_intent_update(intent_hash, version, tx_hash)
            .await?;

        self.state_applicator.apply_state_transition(state_transition).await?;

        Ok(())
    }
}
