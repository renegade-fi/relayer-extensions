//! Defines the application-specific logic for paying the protocol fee accrued
//! on a balance object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::{instrument, warn};

use crate::state_transitions::{StateApplicator, error::StateTransitionError};

// ---------
// | Types |
// ---------

/// A transition representing the payment of the protocol fee accrued on a
/// balance object
#[derive(Clone)]
pub struct PayProtocolFeeTransition {
    /// The now-spent nullifier of the balance on which the protocol fee was
    /// paid
    pub nullifier: Scalar,
    /// The block number in which the protocol fee was paid
    pub block_number: u64,
    /// The public share of the new protocol fee in the balance
    pub new_protocol_fee_public_share: Scalar,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Pay the protocol fee accrued on a balance object
    #[instrument(skip_all, fields(nullifier = %transition.nullifier))]
    pub async fn pay_protocol_fee(
        &self,
        transition: PayProtocolFeeTransition,
        is_backfill: bool,
    ) -> Result<(), StateTransitionError> {
        let PayProtocolFeeTransition { nullifier, block_number, new_protocol_fee_public_share } =
            transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut balance = self.db_client.get_balance_by_nullifier(nullifier, &mut conn).await?;

        balance.update_protocol_fee(new_protocol_fee_public_share);

        conn.transaction(move |conn| {
            async move {
                // Check if the nullifier has already been processed, no-oping if so
                let nullifier_processed =
                    self.db_client.check_nullifier_processed(nullifier, conn).await?;

                if nullifier_processed {
                    warn!(
                        "Nullifier {nullifier} has already been processed, skipping fee payment indexing"
                    );

                    return Ok(());
                }

                // Update the balance record
                self.db_client.update_balance(balance, conn).await?;

                // Mark the nullifier as processed
                self.db_client
                    .mark_nullifier_processed(nullifier, block_number, is_backfill, conn)
                    .await?;

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
                gen_deposit_new_balance_transition, gen_pay_protocol_fee_transition,
                setup_expected_state_object, setup_test_state_applicator,
                validate_balance_indexing,
            },
        },
    };

    /// Test that a protocol fee payment is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_pay_protocol_fee() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        // Index the initial balance creation
        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (create_balance_transition, initial_wrapped_balance) =
            gen_deposit_new_balance_transition(&expected_state_object);

        test_applicator
            .create_balance(create_balance_transition.clone(), false /* is_backfill */)
            .await?;

        // Generate the subsequent fee payment transition
        let (pay_fees_transition, updated_wrapped_balance) =
            gen_pay_protocol_fee_transition(&initial_wrapped_balance);

        // Index the fee payment
        test_applicator
            .pay_protocol_fee(pay_fees_transition.clone(), false /* is_backfill */)
            .await?;

        validate_balance_indexing(db_client, &updated_wrapped_balance).await?;

        // Assert that the nullifier is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(
            db_client.check_nullifier_processed(pay_fees_transition.nullifier, &mut conn).await?
        );

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
