//! Defines the application-specific logic for creating a new public intent

use alloy::primitives::B256;
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::Amount;
use renegade_darkpool_types::intent::Intent;
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::PublicIntentStateObject,
};

// ---------
// | Types |
// ---------

/// A transition representing the creation of a new public intent
#[derive(Clone)]
pub struct CreatePublicIntentTransition {
    /// The public intent to create
    pub intent: Intent,
    /// The input amount on the obligation bundle
    pub amount_in: Amount,
    /// The intent hash
    pub intent_hash: B256,
    /// The block number in which the public intent was created
    pub block_number: u64,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Create a new public intent
    pub async fn create_public_intent(
        &self,
        transition: CreatePublicIntentTransition,
    ) -> Result<(), StateTransitionError> {
        let CreatePublicIntentTransition { mut intent, amount_in, intent_hash, block_number } =
            transition;

        intent.amount_in -= amount_in;

        let mut conn = self.db_client.get_db_conn().await?;
        let master_view_seed =
            self.db_client.get_master_view_seed_by_owner_address(intent.owner, &mut conn).await?;

        let public_intent =
            PublicIntentStateObject::new(intent_hash, intent, master_view_seed.account_id);

        conn.transaction(move |conn| {
            async move {
                // Check if the public intent creation has already been processed, no-oping if so
                let public_intent_creation_processed = self.db_client.check_public_intent_update_processed(intent_hash, public_intent.version, conn).await?;

                if public_intent_creation_processed {
                    warn!(
                        "Public intent creation for intent hash {intent_hash} has already been processed, skipping creation"
                    );

                    return Ok(());
                }

                // Mark the public intent creation as processed
                self.db_client.mark_public_intent_update_processed(intent_hash, public_intent.version, block_number, conn).await?;

                // Check if a public intent record already exists for the intent hash.
                // This is possible in the case that we previously processed a metadata update message for the public intent.
                let public_intent_exists = self.db_client.public_intent_exists(intent_hash, conn).await?;
                if public_intent_exists {
                    warn!(
                        "Public intent record already exists for intent hash {intent_hash}, skipping creation"
                    );

                    return Ok(());
                }

                // Insert the new public intent record
                self.db_client.create_public_intent(public_intent, conn).await?;

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
                gen_create_public_intent_transition, register_random_master_view_seed,
                setup_test_state_applicator, validate_public_intent_indexing,
            },
        },
    };

    /// Test that a public intent creation is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_public_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let transition = gen_create_public_intent_transition(master_view_seed.owner_address);

        let intent_hash = transition.intent_hash;
        let mut intent = transition.intent.clone();
        intent.amount_in -= transition.amount_in;

        // Index the public intent creation
        test_applicator.create_public_intent(transition).await?;

        validate_public_intent_indexing(db_client, intent_hash, &intent).await?;

        // Assert that the public intent creation was marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(
            db_client
                .check_public_intent_update_processed(intent_hash, 0 /* version */, &mut conn)
                .await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
