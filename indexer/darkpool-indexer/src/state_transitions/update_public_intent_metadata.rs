//! Defines the application-specific logic for updating a public intent's
//! metadata, with upsert semantics (creates if not exists, updates if exists)

use darkpool_indexer_api::types::message_queue::PublicIntentMetadataUpdateMessage;
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use tracing::instrument;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::PublicIntentStateObject,
};

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Update a public intent's metadata (creates if not exists, updates if
    /// exists)
    #[instrument(skip_all, fields(intent_hash = %message.intent_hash))]
    pub async fn update_public_intent_metadata(
        &self,
        message: PublicIntentMetadataUpdateMessage,
    ) -> Result<(), StateTransitionError> {
        // Note: This transition does not have an idempotency guard. The relayer is
        // expected to send each metadata update at most once. If idempotency
        // becomes necessary in the future, a deduplication mechanism should be
        // added here.
        let intent_hash = message.intent_hash;
        let owner = message.order.intent.inner.owner;

        let mut conn = self.db_client.get_db_conn().await?;

        conn.transaction(move |conn| {
            async move {
                // Upsert: check if public intent exists and branch accordingly
                let public_intent_exists =
                    self.db_client.public_intent_exists(intent_hash, conn).await?;

                if public_intent_exists {
                    // Update existing: update only metadata fields
                    let mut public_intent =
                        self.db_client.get_public_intent_by_hash(intent_hash, conn).await?;

                    public_intent.update_metadata(&message);
                    self.db_client.update_public_intent(public_intent, conn).await?;
                } else {
                    // Create new: construct from metadata update message
                    let master_view_seed =
                        self.db_client.get_master_view_seed_by_owner_address(owner, conn).await?;

                    let public_intent = PublicIntentStateObject::from_metadata_update_message(
                        &message,
                        master_view_seed.account_id,
                    )
                    .map_err(StateTransitionError::Conversion)?;

                    self.db_client.create_public_intent(public_intent, conn).await?;
                }

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
            settle_public_intent::PublicIntentSettlementData,
            test_utils::{
                gen_public_intent_metadata_update_message,
                gen_public_intent_metadata_update_message_for_existing,
                gen_settle_public_intent_transition, register_random_master_view_seed,
                setup_test_state_applicator, validate_public_intent_indexing,
                validate_public_intent_metadata,
            },
        },
    };

    /// Test that a metadata update creates a new public intent when one doesn't
    /// exist
    #[tokio::test(flavor = "multi_thread")]
    async fn test_update_public_intent_metadata_creates_new() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;
        let message = gen_public_intent_metadata_update_message(master_view_seed.owner_address);

        let intent_hash = message.intent_hash;
        let expected_intent = message.order.intent.inner.clone();
        let expected_order_id = message.order.id;
        let expected_matching_pool = message.matching_pool.clone();
        let expected_allow_external_matches = message.order.metadata.allow_external_matches;
        let expected_min_fill_size = message.order.metadata.min_fill_size;

        // Apply the metadata update (should create since intent doesn't exist)
        test_applicator.update_public_intent_metadata(message).await?;

        // Validate the public intent was created with correct intent fields
        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Validate metadata fields match the message
        validate_public_intent_metadata(
            db_client,
            intent_hash,
            expected_order_id,
            &expected_matching_pool,
            expected_allow_external_matches,
            expected_min_fill_size,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a metadata update only updates metadata fields on an existing
    /// public intent
    #[tokio::test(flavor = "multi_thread")]
    async fn test_update_public_intent_metadata_updates_existing()
    -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let master_view_seed = register_random_master_view_seed(&test_applicator).await?;

        // First, create a public intent via settle
        let create_transition = gen_settle_public_intent_transition(master_view_seed.owner_address);
        let intent_hash = create_transition.intent_hash;

        let PublicIntentSettlementData::InternalMatch { intent: initial_intent, amount_in, .. } =
            &create_transition.public_intent_settlement_data
        else {
            panic!("Expected InternalMatch variant");
        };

        let mut expected_intent = initial_intent.clone();
        expected_intent.amount_in -= amount_in;

        test_applicator.settle_public_intent(create_transition, false /* is_backfill */).await?;

        // Get the existing public intent to generate an update message
        let mut conn = db_client.get_db_conn().await?;
        let existing_public_intent =
            db_client.get_public_intent_by_hash(intent_hash, &mut conn).await?;

        // Generate a metadata update message with different metadata values
        let update_message =
            gen_public_intent_metadata_update_message_for_existing(&existing_public_intent);

        let expected_order_id = update_message.order.id;
        let expected_matching_pool = update_message.matching_pool.clone();
        let expected_allow_external_matches = update_message.order.metadata.allow_external_matches;
        let expected_min_fill_size = update_message.order.metadata.min_fill_size;

        // Apply the metadata update (should update existing)
        test_applicator.update_public_intent_metadata(update_message).await?;

        // Validate the core intent fields remain unchanged
        validate_public_intent_indexing(db_client, intent_hash, &expected_intent).await?;

        // Validate only metadata fields changed
        validate_public_intent_metadata(
            db_client,
            intent_hash,
            expected_order_id,
            &expected_matching_pool,
            expected_allow_external_matches,
            expected_min_fill_size,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
