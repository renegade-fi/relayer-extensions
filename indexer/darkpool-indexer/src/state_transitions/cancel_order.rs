//! Defines the application-specific logic for canceling an order

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::warn;

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

// ---------
// | Types |
// ---------

/// A transition representing the cancellation of an order
#[derive(Clone)]
pub struct CancelOrderTransition {
    /// The now-spent nullifier of the canceled order
    pub nullifier: Scalar,
    /// The block number in which the order was canceled
    pub block_number: u64,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Cancel an order
    pub async fn cancel_order(
        &self,
        transition: CancelOrderTransition,
    ) -> Result<(), StateTransitionError> {
        let CancelOrderTransition { nullifier, block_number } = transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut intent = self.db_client.get_intent_by_nullifier(nullifier, &mut conn).await?;

        intent.cancel();

        conn.transaction(move |conn| {
            async move {
                // Check if the nullifier has already been processed, no-oping if so
                let nullifier_processed =
                    self.db_client.check_nullifier_processed(nullifier, conn).await?;
        
                if nullifier_processed {
                    warn!(
                        "Nullifier {nullifier} has already been processed, skipping order cancellation indexing"
                    );

                    return Ok(());
                }

                // Update the intent record
                self.db_client.update_intent(intent, conn).await?;

                // Mark the nullifier as processed
                self.db_client.mark_nullifier_processed(nullifier, block_number, conn).await?;

                Ok(())
            }.scope_boxed()
        }).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{db::test_utils::cleanup_test_db, state_transitions::{error::StateTransitionError, test_utils::{gen_cancel_order_transition, gen_create_intent_transition, setup_expected_state_object, setup_test_state_applicator}}};

    /// Test that an order cancellation is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_cancel_order() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        // Index the initial intent creation
        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (create_intent_transition, initial_wrapped_intent) =
            gen_create_intent_transition(&expected_state_object);

        test_applicator.create_intent(create_intent_transition.clone()).await?;

        // Generate the subsequent order cancellation transition
        let cancel_order_transition =
            gen_cancel_order_transition(&initial_wrapped_intent);

        // Index the order cancellation
        test_applicator.cancel_order(cancel_order_transition.clone()).await?;

        // Assert that the intent is no longer active
        let mut conn = db_client.get_db_conn().await?;
        let nullifier = initial_wrapped_intent.compute_nullifier();
        let intent = db_client.get_intent_by_nullifier(nullifier, &mut conn).await?;
        assert!(!intent.active);

        // Assert that the nullifier is marked as processed
        assert!(db_client.check_nullifier_processed(nullifier, &mut conn).await?);

        cleanup_test_db(postgres).await?;

        Ok(())
    }
}
