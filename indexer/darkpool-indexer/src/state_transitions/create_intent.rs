//! Defines the application-specific logic for creating a new intent object

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::intent::{IntentShare, PreMatchIntentShare};
use renegade_constants::Scalar;
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
    /// The data needed to construct the intent shares for a natively-settled
    /// private intent
    NativelySettledPrivateIntent {
        /// The full sharing of the intent before the settlement was applied to
        /// it
        pre_match_full_intent_share: IntentShare,
        /// The input amount on the obligation bundle
        amount_in: Scalar,
    },
    /// A complete set of updated intent public shares, available in
    /// Renegade-settled private-fill matches
    RenegadeSettledPrivateFill(IntentShare),
    /// The data needed to construct the intent shares for a Renegade-settled
    /// public-fill match, where only the pre-match amount public share is
    /// leaked
    RenegadeSettledPublicFill {
        /// The pre-match intent public shares, *excluding* the updated amount
        /// public share
        pre_match_partial_intent_share: PreMatchIntentShare,
        /// The pre-match amount public share
        pre_match_amount_share: Scalar,
        /// The input amount on the obligation bundle
        amount_in: Scalar,
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

        let public_share = get_new_intent_shares(intent_creation_data);

        let IntentCreationPrestate { expected_state_object, mut master_view_seed } =
            self.get_intent_creation_prestate(recovery_id).await?;

        let ExpectedStateObject {
            recovery_id,
            recovery_stream_seed,
            share_stream_seed,
            account_id,
        } = expected_state_object;

        let intent = IntentStateObject::new(
            public_share,
            recovery_stream_seed,
            share_stream_seed,
            account_id,
        );

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
fn get_new_intent_shares(intent_creation_data: IntentCreationData) -> IntentShare {
    match intent_creation_data {
        IntentCreationData::NativelySettledPrivateIntent {
            mut pre_match_full_intent_share,
            amount_in,
        } => {
            pre_match_full_intent_share.amount_in -= amount_in;
            pre_match_full_intent_share
        },
        IntentCreationData::RenegadeSettledPrivateFill(updated_intent_share) => {
            updated_intent_share
        },
        IntentCreationData::RenegadeSettledPublicFill {
            pre_match_partial_intent_share: pre_match_intent_shares,
            pre_match_amount_share,
            amount_in: obligation_amount_in,
        } => {
            let updated_amount_share = pre_match_amount_share - obligation_amount_in;
            from_pre_match_intent_and_amount(pre_match_intent_shares, updated_amount_share)
        },
    }
}

/// Construct a circuit `IntentShare` from a `PreMatchIntentShare` and an amount
pub fn from_pre_match_intent_and_amount(
    pre_match_intent_share: PreMatchIntentShare,
    amount_in: Scalar,
) -> IntentShare {
    let PreMatchIntentShare { in_token, out_token, owner, min_price } = pre_match_intent_share;

    IntentShare { in_token, out_token, owner, min_price, amount_in }
}

#[cfg(test)]
mod tests {
    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::test_utils::{
            gen_create_intent_transition, setup_expected_state_object, setup_test_state_applicator,
            validate_expected_state_object_rotation, validate_intent_indexing,
        },
    };

    use super::*;

    /// Test that an intent creation is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_intent) = gen_create_intent_transition(&expected_state_object);

        // Index the intent creation
        test_applicator.create_intent(transition.clone()).await?;

        validate_intent_indexing(db_client, &wrapped_intent).await?;

        validate_expected_state_object_rotation(db_client, &expected_state_object).await?;

        // Assert that the recovery ID is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_recovery_id_processed(transition.recovery_id, &mut conn).await?);

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
