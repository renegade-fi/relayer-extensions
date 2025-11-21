//! Defines the application-specific logic for settling a match into a balance
//! object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::warn;

use crate::{state_transitions::{StateApplicator, error::StateTransitionError}, types::{BalanceSharesInMatch, ObligationAmounts}};

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
    /// A balance update resulting from a public fill being settled
    PublicFill {
        /// The pre-update balance shares affected by the match
        pre_update_balance_shares: BalanceSharesInMatch,
        /// The input/output amounts parsed from the public obligation bundle
        obligation_amounts: ObligationAmounts,
        /// Whether the balance being settled into is the input balance
        is_input_balance: bool,
    },
    /// A balance update resulting from a private fill being settled. Contains the updated balance shares resulting from the match settlement.
    PrivateFill(BalanceSharesInMatch),
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
            balance_settlement_data,
        } = transition;

        let updated_balance_shares = get_updated_balance_public_shares(balance_settlement_data);

        let mut conn = self.db_client.get_db_conn().await?;
        let mut balance = self.db_client.get_balance_by_nullifier(nullifier, &mut conn).await?;

        balance.update_from_match(updated_balance_shares);

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

/// Get the updated balance shares resulting from a match settlement
fn get_updated_balance_public_shares(balance_settlement_data: BalanceSettlementData) -> BalanceSharesInMatch {
    match balance_settlement_data {
        BalanceSettlementData::PublicFill { pre_update_balance_shares, obligation_amounts, is_input_balance } => {
            let BalanceSharesInMatch { relayer_fee_public_share, protocol_fee_public_share, mut amount_public_share } = pre_update_balance_shares;

            let ObligationAmounts { amount_in, amount_out } = obligation_amounts;

            if is_input_balance {
                amount_public_share -= amount_in;
            } else {
                amount_public_share += amount_out;
            }

            // TODO: Account for fee accrual & one-time authority rotation

            BalanceSharesInMatch { relayer_fee_public_share, protocol_fee_public_share, amount_public_share }
        },
        BalanceSettlementData::PrivateFill (updated_balance_shares) => updated_balance_shares,
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
