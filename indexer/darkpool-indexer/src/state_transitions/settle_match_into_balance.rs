//! Defines the application-specific logic for settling a match into a balance
//! object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::warn;

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

// ---------
// | Types |
// ---------

/// A transition representing the settlement of a match into a balance object
#[derive(Clone)]
pub struct SettleMatchIntoBalanceTransition {
    /// The now-spent nullifier of the balance being settled into
    pub nullifier: Scalar,
    /// The block number in which the match was settled
    pub block_number: u64,
    /// The public share of the new relayer fee in the balance
    pub new_relayer_fee_public_share: Scalar,
    /// The public share of the new protocol fee in the balance
    pub new_protocol_fee_public_share: Scalar,
    /// The public share of the new amount in the balance
    pub new_amount_public_share: Scalar,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Settle a match into a balance object
    pub async fn settle_match_into_balance(
        &self,
        transition: SettleMatchIntoBalanceTransition,
    ) -> Result<(), StateTransitionError> {
        let SettleMatchIntoBalanceTransition {
            nullifier,
            block_number,
            new_relayer_fee_public_share,
            new_protocol_fee_public_share,
            new_amount_public_share,
        } = transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut balance = self.db_client.get_balance_by_nullifier(nullifier, &mut conn).await?;

        balance.update_amount_and_fees(new_relayer_fee_public_share, new_protocol_fee_public_share, new_amount_public_share);

        conn.transaction(move |conn| {
            async move {
                // Check if the nullifier has already been processed, no-oping if so
                let nullifier_processed =
                    self.db_client.check_nullifier_processed(nullifier, conn).await?;
        
                if nullifier_processed {
                    warn!(
                        "Nullifier {nullifier} has already been processed, skipping indexing of match settlement into balance"
                    );

                    return Ok(());
                }

                // Update the balance record
                self.db_client.update_balance(balance, conn).await?;

                // Mark the nullifier as processed
                self.db_client.mark_nullifier_processed(nullifier, block_number, conn).await?;

                Ok(())
                
            }.scope_boxed()
        }).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{db::test_utils::cleanup_test_db, state_transitions::{error::StateTransitionError, test_utils::{gen_create_balance_transition, gen_settle_match_into_balance_transition, setup_expected_state_object, setup_test_state_applicator, validate_balance_indexing}}};

    /// Test that a match settlement into a balance is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_match_into_balance() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        // Index the initial balance creation
        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (create_balance_transition, initial_wrapped_balance) =
            gen_create_balance_transition(&expected_state_object);

        test_applicator.create_balance(create_balance_transition.clone()).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_balance_transition, updated_wrapped_balance) =
            gen_settle_match_into_balance_transition(&initial_wrapped_balance);

        // Index the match settlement
        test_applicator.settle_match_into_balance(settle_match_into_balance_transition.clone()).await?;

        validate_balance_indexing(db_client, &updated_wrapped_balance).await?;

        // Assert that the nullifier is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_nullifier_processed(settle_match_into_balance_transition.nullifier, &mut conn).await?);

        cleanup_test_db(postgres).await?;

        Ok(())
    }
}
