//! Defines the application-specific logic for settling a match into a balance
//! object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::{
    balance::PostMatchBalanceShare, fee::FeeTake, fixed_point::FixedPoint,
    settlement_obligation::SettlementObligation,
};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::BalanceStateObject,
};

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
    /// The data required to update a balance resulting from a match settlement
    pub balance_settlement_data: BalanceSettlementData,
}

/// The data required to update a balance resulting from a match settlement
#[derive(Clone)]
pub enum BalanceSettlementData {
    /// An input balance update resulting from a public fill being settled
    PublicFillInputBalance {
        /// The settlement obligation for the fill
        settlement_obligation: SettlementObligation,
    },
    /// An output balance update resulting from a public fill being settled
    PublicFillOutputBalance {
        /// The settlement obligation for the fill
        settlement_obligation: SettlementObligation,
        /// The relayer fee rate used in the fill
        relayer_fee_rate: FixedPoint,
        /// The protocol fee rate used in the fill
        protocol_fee_rate: FixedPoint,
    },
    /// A balance update resulting from a private fill being settled. Contains
    /// the updated balance shares resulting from the match settlement.
    PrivateFill(PostMatchBalanceShare),
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
        let SettleMatchIntoBalanceTransition { nullifier, block_number, balance_settlement_data } =
            transition;

        let mut conn = self.db_client.get_db_conn().await?;
        let mut balance = self.db_client.get_balance_by_nullifier(nullifier, &mut conn).await?;

        apply_settlement_into_balance(balance_settlement_data, &mut balance);

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

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Apply a match settlement into the balance
fn apply_settlement_into_balance(
    balance_settlement_data: BalanceSettlementData,
    balance: &mut BalanceStateObject,
) {
    match balance_settlement_data {
        BalanceSettlementData::PublicFillInputBalance { settlement_obligation } => {
            balance.update_from_public_fill_as_input_balance(&settlement_obligation)
        },
        BalanceSettlementData::PublicFillOutputBalance {
            settlement_obligation,
            relayer_fee_rate,
            protocol_fee_rate,
        } => {
            // Compute the fee take
            let receive_amount = settlement_obligation.amount_out;

            let relayer_fee = scalar_to_u128(&relayer_fee_rate.floor_mul_int(receive_amount));
            let protocol_fee = scalar_to_u128(&protocol_fee_rate.floor_mul_int(receive_amount));

            let fee_take = FeeTake { relayer_fee, protocol_fee };

            balance.update_from_public_fill_as_output_balance(&settlement_obligation, &fee_take);
        },
        BalanceSettlementData::PrivateFill(updated_balance_shares) => {
            balance.update_from_private_fill(&updated_balance_shares);
        },
    }
}

#[cfg(test)]
mod tests {
    use renegade_circuit_types::balance::DarkpoolStateBalance;

    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::{
            StateApplicator,
            error::StateTransitionError,
            settle_match_into_balance::SettleMatchIntoBalanceTransition,
            test_utils::{
                gen_deposit_new_balance_transition,
                gen_settle_private_fill_into_balance_transition,
                gen_settle_public_fill_into_input_balance_transition,
                gen_settle_public_fill_into_output_balance_transition, setup_expected_state_object,
                setup_test_state_applicator, validate_balance_indexing,
            },
        },
    };

    /// Set up an initial balance for testing
    async fn setup_initial_balance(
        test_applicator: &StateApplicator,
    ) -> Result<DarkpoolStateBalance, StateTransitionError> {
        // Index the initial balance creation
        let expected_state_object = setup_expected_state_object(test_applicator).await?;
        let (create_balance_transition, wrapped_balance) =
            gen_deposit_new_balance_transition(&expected_state_object);

        test_applicator.create_balance(create_balance_transition).await?;

        Ok(wrapped_balance)
    }

    /// Index a match settlement into a balance and validate the indexing
    async fn validate_settle_match_into_balance_indexing(
        test_applicator: &StateApplicator,
        transition: SettleMatchIntoBalanceTransition,
        updated_wrapped_balance: &DarkpoolStateBalance,
    ) -> Result<(), StateTransitionError> {
        let db_client = &test_applicator.db_client;
        let nullifier = transition.nullifier;

        // Index the match settlement
        test_applicator.settle_match_into_balance(transition).await?;

        validate_balance_indexing(db_client, updated_wrapped_balance).await?;

        // Assert that the nullifier is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_nullifier_processed(nullifier, &mut conn).await?);

        Ok(())
    }

    /// Test that a private-fill match settlement into a balance is
    /// indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_private_fill_into_balance() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let initial_wrapped_balance = setup_initial_balance(&test_applicator).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_balance_transition, updated_wrapped_balance) =
            gen_settle_private_fill_into_balance_transition(&initial_wrapped_balance);

        validate_settle_match_into_balance_indexing(
            &test_applicator,
            settle_match_into_balance_transition,
            &updated_wrapped_balance,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a public-fill match settlement into an input balance is
    /// indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_fill_into_input_balance() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let initial_wrapped_balance = setup_initial_balance(&test_applicator).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_balance_transition, updated_wrapped_balance) =
            gen_settle_public_fill_into_input_balance_transition(&initial_wrapped_balance);

        validate_settle_match_into_balance_indexing(
            &test_applicator,
            settle_match_into_balance_transition,
            &updated_wrapped_balance,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a public-fill match settlement into an output balance is
    /// indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_settle_public_fill_into_output_balance() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let initial_wrapped_balance = setup_initial_balance(&test_applicator).await?;

        // Generate the subsequent match settlement transition
        let (settle_match_into_balance_transition, updated_wrapped_balance) =
            gen_settle_public_fill_into_output_balance_transition(&initial_wrapped_balance);

        validate_settle_match_into_balance_indexing(
            &test_applicator,
            settle_match_into_balance_transition,
            &updated_wrapped_balance,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
