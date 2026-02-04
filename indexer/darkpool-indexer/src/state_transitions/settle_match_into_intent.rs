//! Defines the application-specific logic for settling a match into an intent
//! object

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use renegade_darkpool_types::settlement_obligation::SettlementObligation;
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::IntentStateObject,
};

// ---------
// | Types |
// ---------

/// A transition representing the settlement of a match into an intent object
#[derive(Clone)]
pub struct SettleMatchIntoIntentTransition {
    /// The now-spent nullifier of the intent being settled into
    pub nullifier: Scalar,
    /// The block number in which the match was settled
    pub block_number: u64,
    /// The data required to update an intent resulting from a match settlement
    pub intent_settlement_data: IntentSettlementData,
}

/// The data required to update an intent resulting from a match settlement
#[derive(Clone)]
pub enum IntentSettlementData {
    /// The post-match public share of the intent amount
    UpdatedAmountShare(Scalar),
    /// The data needed to update the intent for a public-fill match
    /// (either natively-settled or Renegade-settled)
    PublicFill {
        /// The settlement obligation for the fill
        settlement_obligation: SettlementObligation,
    },
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Settle a match into an intent object
    pub async fn settle_match_into_intent(
        &self,
        transition: SettleMatchIntoIntentTransition,
        is_backfill: bool,
    ) -> Result<(), StateTransitionError> {
        let SettleMatchIntoIntentTransition { nullifier, block_number, intent_settlement_data } =
            transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut intent = self.db_client.get_intent_by_nullifier(nullifier, &mut conn).await?;

        apply_settlement_into_intent(intent_settlement_data, &mut intent);

        conn.transaction(move |conn| {
            async move {
                // Check if the nullifier has already been processed, no-oping if so
                let nullifier_processed =
                    self.db_client.check_nullifier_processed(nullifier, conn).await?;

                if nullifier_processed {
                    warn!(
                        "Nullifier {nullifier} has already been processed, skipping indexing of match settlement into intent"
                    );

                    return Ok(());
                }

                // Update the intent record
                self.db_client.update_intent(intent, conn).await?;

                // Mark the nullifier as processed
                self.db_client
                    .mark_nullifier_processed(nullifier, block_number, is_backfill, conn)
                    .await?;

                Ok(())
            }.scope_boxed()
        }).await
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Apply a match settlement into the intent
fn apply_settlement_into_intent(
    intent_settlement_data: IntentSettlementData,
    intent: &mut IntentStateObject,
) {
    match intent_settlement_data {
        IntentSettlementData::UpdatedAmountShare(updated_amount_share) => {
            intent.update_amount(updated_amount_share)
        },
        IntentSettlementData::PublicFill { settlement_obligation } => {
            intent.update_from_settlement_obligation(&settlement_obligation)
        },
    }
}

#[cfg(test)]
mod tests {
    use renegade_darkpool_types::intent::DarkpoolStateIntent;

    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            StateApplicator,
            error::StateTransitionError,
            settle_match_into_intent::SettleMatchIntoIntentTransition,
            test_utils::{
                gen_create_intent_from_private_fill_transition,
                gen_settle_match_into_intent_transition,
                gen_settle_public_fill_into_intent_transition, setup_expected_state_object,
                setup_test_state_applicator, validate_intent_indexing,
            },
        },
    };

    /// Set up an initial intent for testing
    async fn setup_initial_intent(
        test_applicator: &StateApplicator,
    ) -> Result<DarkpoolStateIntent, StateTransitionError> {
        // Index the initial intent creation
        let expected_state_object = setup_expected_state_object(test_applicator).await?;
        let (create_intent_transition, wrapped_intent) =
            gen_create_intent_from_private_fill_transition(&expected_state_object);

        test_applicator.create_intent(create_intent_transition, false /* is_backfill */).await?;

        Ok(wrapped_intent)
    }

    /// Index a match settlement into an intent and validate the indexing
    async fn validate_settle_match_into_intent_indexing(
        test_applicator: &StateApplicator,
        transition: SettleMatchIntoIntentTransition,
        updated_wrapped_intent: &DarkpoolStateIntent,
    ) -> Result<(), StateTransitionError> {
        let db_client = &test_applicator.db_client;
        let nullifier = transition.nullifier;

        // Index the match settlement
        test_applicator.settle_match_into_intent(transition, false /* is_backfill */).await?;

        validate_intent_indexing(db_client, updated_wrapped_intent).await?;

        // Assert that the nullifier is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_nullifier_processed(nullifier, &mut conn).await?);

        Ok(())
    }

    /// Test that indexing a match settlement (specifically, *not* a
    /// Renegade-settled public-fill match) into an intent is done correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_basic_match_into_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let initial_wrapped_intent = setup_initial_intent(&test_applicator).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_intent_transition, updated_wrapped_intent) =
            gen_settle_match_into_intent_transition(&initial_wrapped_intent);

        validate_settle_match_into_intent_indexing(
            &test_applicator,
            settle_match_into_intent_transition,
            &updated_wrapped_intent,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that indexing a Renegade-settled public-fill match settlement into
    /// an intent is done correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_fill_into_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let initial_wrapped_intent = setup_initial_intent(&test_applicator).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_intent_transition, updated_wrapped_intent) =
            gen_settle_public_fill_into_intent_transition(&initial_wrapped_intent);

        validate_settle_match_into_intent_indexing(
            &test_applicator,
            settle_match_into_intent_transition,
            &updated_wrapped_intent,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
