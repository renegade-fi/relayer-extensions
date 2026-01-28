//! Defines the application-specific logic for creating a new intent object

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use renegade_darkpool_types::{intent::IntentShare, settlement_obligation::SettlementObligation};
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::{ExpectedStateObject, IntentStateObject, MasterViewSeed},
};

// ---------
// | Types |
// ---------

/// A transition representing the creation of a new intent object
#[derive(Clone)]
pub struct CreateIntentTransition {
    /// The recovery ID registered for the intent
    pub recovery_id: Scalar,
    /// The block number in which the recovery ID was registered
    pub block_number: u64,
    /// The data required to create a new intent object
    pub intent_creation_data: IntentCreationData,
}

/// The data required to create a new intent object
#[derive(Clone)]
pub enum IntentCreationData {
    /// A complete set of updated intent public shares, available in
    /// Renegade-settled private-fill matches
    RenegadeSettledPrivateFill(IntentShare),
    /// The data needed to create a new intent object for a public-fill match
    /// (either natively-settled or Renegade-settled)
    PublicFill {
        /// The full sharing of the intent before the settlement was applied to
        /// it
        pre_match_full_intent_share: IntentShare,
        /// The settlement obligation
        settlement_obligation: SettlementObligation,
    },
}

/// The pre-state required for the creation of a new intent object
struct IntentCreationPrestate {
    /// The expected state object that will be replaced by the created intent
    expected_state_object: ExpectedStateObject,
    /// The master view seed of the account owning the intent
    master_view_seed: MasterViewSeed,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Create a new intent object
    pub async fn create_intent(
        &self,
        transition: CreateIntentTransition,
    ) -> Result<(), StateTransitionError> {
        let CreateIntentTransition { recovery_id, block_number, intent_creation_data } = transition;

        let IntentCreationPrestate { expected_state_object, mut master_view_seed } =
            self.get_intent_creation_prestate(recovery_id).await?;

        let recovery_id = expected_state_object.recovery_id;
        let recovery_stream_seed = expected_state_object.recovery_stream_seed;

        let intent = construct_new_intent(intent_creation_data, &expected_state_object);

        let next_expected_state_object = master_view_seed.next_expected_state_object();

        let mut conn = self.db_client.get_db_conn().await?;
        conn.transaction(move |conn| {
            async move {
                // Check if the recovery ID has already been processed, no-oping if so
                let recovery_id_processed =
                    self.db_client.check_recovery_id_processed(recovery_id, conn).await?;

                if recovery_id_processed {
                    warn!(
                        "Recovery ID {recovery_id} has already been processed, skipping intent creation"
                    );

                    return Ok(());
                }

                // Mark the recovery ID as processed
                self.db_client.mark_recovery_id_processed(recovery_id, block_number, conn).await?;

                // Check if an intent record already exists for the recovery stream seed.
                // This is possible in the case that we previously processed a metadata update message for the intent.
                let intent_exists = self.db_client.intent_exists(recovery_stream_seed, conn).await?;
                if intent_exists {
                    // We assume the intent details with which the record was originally inserted
                    // match those derived from the public shares
                    warn!("Intent record already exists for recovery stream seed {recovery_stream_seed}, skipping creation");
                    return Ok(());
                }

                // Insert the new intent record
                self.db_client.create_intent(intent, conn).await?;

                // Delete the now-created expected state object, insert the next one,
                // & update the master view seed as its CSPRNG states have advanced
                self.db_client.delete_expected_state_object(recovery_id, conn).await?;
                self.db_client.insert_expected_state_object(next_expected_state_object, conn).await?;
                self.db_client.update_master_view_seed(master_view_seed, conn).await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    /// Get the pre-state required for the creation of a new intent object
    async fn get_intent_creation_prestate(
        &self,
        recovery_id: Scalar,
    ) -> Result<IntentCreationPrestate, StateTransitionError> {
        let mut conn = self.db_client.get_db_conn().await?;
        conn.transaction(|conn| {
            async move {
                let expected_state_object =
                    self.db_client.get_expected_state_object(recovery_id, conn).await?;

                let master_view_seed = self
                    .db_client
                    .get_master_view_seed_by_account_id(expected_state_object.account_id, conn)
                    .await?;

                Ok(IntentCreationPrestate { expected_state_object, master_view_seed })
            }
            .scope_boxed()
        })
        .await
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Get the new intent shares from the intent creation data
fn construct_new_intent(
    intent_creation_data: IntentCreationData,
    expected_state_object: &ExpectedStateObject,
) -> IntentStateObject {
    let ExpectedStateObject { recovery_stream_seed, share_stream_seed, account_id, .. } =
        expected_state_object;

    match intent_creation_data {
        IntentCreationData::RenegadeSettledPrivateFill(full_intent_share) => {
            IntentStateObject::new(
                full_intent_share,
                *recovery_stream_seed,
                *share_stream_seed,
                *account_id,
            )
        },
        IntentCreationData::PublicFill { pre_match_full_intent_share, settlement_obligation } => {
            let mut intent = IntentStateObject::new(
                pre_match_full_intent_share,
                *recovery_stream_seed,
                *share_stream_seed,
                *account_id,
            );

            intent.intent.apply_settlement_obligation(&settlement_obligation);

            // Note: We don't re-encrypt the updated intent amount share

            intent
        },
    }
}

#[cfg(test)]
mod tests {
    use renegade_darkpool_types::intent::DarkpoolStateIntent;

    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::test_utils::{
            gen_create_intent_from_private_fill_transition,
            gen_create_intent_from_public_fill_transition, setup_expected_state_object,
            setup_test_state_applicator, validate_expected_state_object_rotation,
            validate_intent_indexing,
        },
    };

    use super::*;

    /// Index an intent creation and validate the indexing
    async fn validate_intent_creation_indexing(
        test_applicator: &StateApplicator,
        transition: CreateIntentTransition,
        wrapped_intent: &DarkpoolStateIntent,
        expected_state_object: &ExpectedStateObject,
    ) -> Result<(), StateTransitionError> {
        let db_client = &test_applicator.db_client;
        let recovery_id = transition.recovery_id;

        // Index the intent creation
        test_applicator.create_intent(transition).await?;

        validate_intent_indexing(db_client, wrapped_intent).await?;

        validate_expected_state_object_rotation(db_client, expected_state_object).await?;

        // Assert that the recovery ID is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_recovery_id_processed(recovery_id, &mut conn).await?);

        Ok(())
    }

    /// Test that an intent creation from a private fill is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_intent_from_private_fill() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_intent) =
            gen_create_intent_from_private_fill_transition(&expected_state_object);

        validate_intent_creation_indexing(
            &test_applicator,
            transition,
            &wrapped_intent,
            &expected_state_object,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that an intent creation from a public fill is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_intent_from_public_fill() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_intent) =
            gen_create_intent_from_public_fill_transition(&expected_state_object);

        validate_intent_creation_indexing(
            &test_applicator,
            transition,
            &wrapped_intent,
            &expected_state_object,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
