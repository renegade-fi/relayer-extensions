//! Defines the application-specific logic for settling a match into a public
//! intent

use alloy::primitives::B256;
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::Amount;
use tracing::warn;

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

// ---------
// | Types |
// ---------

/// A transition representing the settlement of a match into a public intent
#[derive(Clone)]
pub struct SettleMatchIntoPublicIntentTransition {
    /// The input amount on the obligation bundle
    pub amount_in: Amount,
    /// The intent hash
    pub intent_hash: B256,
    /// The post-match version of the public intent
    pub version: u64,
    /// The block number in which the match settled
    pub block_number: u64,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Settle a match into a public intent
    pub async fn settle_match_into_public_intent(
        &self,
        transition: SettleMatchIntoPublicIntentTransition,
    ) -> Result<(), StateTransitionError> {
        let SettleMatchIntoPublicIntentTransition { amount_in, intent_hash, version, block_number } =
            transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut public_intent =
            self.db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        public_intent.intent.amount_in -= amount_in;
        public_intent.version = version;

        conn.transaction(move |conn| {
            async move {
                // Check if the public intent update has already been processed, no-oping if so
                let public_intent_update_processed = self.db_client.check_public_intent_update_processed(intent_hash, version, conn).await?;

                if public_intent_update_processed {
                    warn!(
                        "Public intent update for intent hash {intent_hash} & version {version} has already been processed, skipping update"
                    );

                    return Ok(());
                }

                // Update the public intent record
                self.db_client.update_public_intent(public_intent, conn).await?;

                // Mark the public intent update as processed
                self.db_client.mark_public_intent_update_processed(intent_hash, version, block_number, conn).await?;

                Ok(())
            }.scope_boxed()
        }).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            error::StateTransitionError,
            test_utils::{
                gen_create_public_intent_transition,
                gen_settle_match_into_public_intent_transition, register_random_master_view_seed,
                setup_test_state_applicator, validate_public_intent_indexing,
            },
        },
    };

    /// Test that a match settlement into a public intent is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_match_into_public_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let create_public_intent_transition =
            gen_create_public_intent_transition(master_view_seed.owner_address);

        let intent_hash = create_public_intent_transition.intent_hash;

        // Index the initial public intent creation
        test_applicator.create_public_intent(create_public_intent_transition.clone()).await?;

        // Generate the subsequent match settlement transition
        let mut conn = db_client.get_db_conn().await?;
        let initial_public_intent =
            db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        let settle_match_into_public_intent_transition =
            gen_settle_match_into_public_intent_transition(&initial_public_intent);

        let mut intent = create_public_intent_transition.intent.clone();
        intent.amount_in -= create_public_intent_transition.amount_in;
        intent.amount_in -= settle_match_into_public_intent_transition.amount_in;

        // Index the match settlement
        test_applicator
            .settle_match_into_public_intent(settle_match_into_public_intent_transition)
            .await?;

        validate_public_intent_indexing(db_client, intent_hash, &intent).await?;

        // Assert that the public intent update was marked as processed

        assert!(
            db_client
                .check_public_intent_update_processed(intent_hash, 1 /* version */, &mut conn)
                .await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
