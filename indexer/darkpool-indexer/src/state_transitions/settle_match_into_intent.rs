//! Defines the application-specific logic for settling a match into an intent
//! object

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::warn;

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

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
#[derive(Clone, Copy)]
pub enum IntentSettlementData {
    /// The post-match public share of the intent amount
    UpdatedAmountShare(Scalar),
    /// The data needed to compute the post-match public share of the intent
    /// amount for a Renegade-settled public-fill match
    RenegadeSettledPublicFill {
        /// The pre-match amount public share
        pre_match_amount_share: Scalar,
        /// The input amount on the obligation bundle
        amount_in: Scalar,
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
    ) -> Result<(), StateTransitionError> {
        let SettleMatchIntoIntentTransition { nullifier, block_number, intent_settlement_data } =
            transition;

        let updated_intent_amount_public_share =
            get_updated_intent_amount_public_share(intent_settlement_data);

        let mut conn = self.db_client.get_db_conn().await?;
        let mut intent = self.db_client.get_intent_by_nullifier(nullifier, &mut conn).await?;

        intent.update_amount(updated_intent_amount_public_share);

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
                self.db_client.mark_nullifier_processed(nullifier, block_number, conn).await?;

                Ok(())
            }.scope_boxed()
        }).await
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Get the post-match public share of the intent amount resulting from a match
/// settlement
fn get_updated_intent_amount_public_share(intent_settlement_data: IntentSettlementData) -> Scalar {
    match intent_settlement_data {
        IntentSettlementData::UpdatedAmountShare(updated_amount_share) => updated_amount_share,
        IntentSettlementData::RenegadeSettledPublicFill { pre_match_amount_share, amount_in } => {
            pre_match_amount_share - amount_in
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::{db::test_utils::cleanup_test_db, state_transitions::{error::StateTransitionError, test_utils::{gen_create_intent_transition, gen_settle_match_into_intent_transition, setup_expected_state_object, setup_test_state_applicator, validate_intent_indexing}}};

    /// Test that a match settlement into an intent is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_match_into_intent() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        // Index the initial intent creation
        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (create_intent_transition, initial_wrapped_intent) =
            gen_create_intent_transition(&expected_state_object);

        test_applicator.create_intent(create_intent_transition.clone()).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_intent_transition, updated_wrapped_intent) =
            gen_settle_match_into_intent_transition(&initial_wrapped_intent);

        // Index the match settlement
        test_applicator.settle_match_into_intent(settle_match_into_intent_transition.clone()).await?;

        validate_intent_indexing(db_client, &updated_wrapped_intent).await?;

        // Assert that the nullifier is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_nullifier_processed(settle_match_into_intent_transition.nullifier, &mut conn).await?);

        cleanup_test_db(postgres).await?;

        Ok(())
    }
}
