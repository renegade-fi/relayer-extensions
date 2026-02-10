//! Defines the application-specific logic for canceling a public intent

use darkpool_indexer_api::types::message_queue::CancelPublicIntentMessage;
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use tracing::instrument;

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Cancel a public intent
    #[instrument(skip_all, fields(intent_hash = %message.intent_hash))]
    pub async fn cancel_public_intent(
        &self,
        message: CancelPublicIntentMessage,
    ) -> Result<(), StateTransitionError> {
        let CancelPublicIntentMessage { intent_hash, .. } = message;

        let mut conn = self.db_client.get_db_conn().await?;

        conn.transaction(|conn| {
            async move {
                let mut public_intent =
                    self.db_client.get_public_intent_by_hash(intent_hash, conn).await?;

                public_intent.cancel();

                self.db_client.update_public_intent(public_intent, conn).await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            error::StateTransitionError,
            test_utils::{
                gen_cancel_public_intent_message, gen_settle_public_intent_transition,
                register_random_master_view_seed, setup_test_state_applicator,
            },
        },
    };

    /// Test that a public intent cancellation is indexed correctly
    #[tokio::test(flavor = "multi_thread")]
    async fn test_cancel_public_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        // First, create a public intent via settle
        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let create_transition = gen_settle_public_intent_transition(master_view_seed.owner_address);
        let intent_hash = create_transition.intent_hash;

        test_applicator.settle_public_intent(create_transition, false /* is_backfill */).await?;

        // Verify the public intent is active
        let mut conn = db_client.get_db_conn().await?;
        let public_intent = db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;
        assert!(public_intent.active);

        // Generate and apply the cancellation
        let cancel_message = gen_cancel_public_intent_message(intent_hash);
        test_applicator.cancel_public_intent(cancel_message).await?;

        // Verify the public intent is now inactive
        let public_intent = db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;
        assert!(!public_intent.active);

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
