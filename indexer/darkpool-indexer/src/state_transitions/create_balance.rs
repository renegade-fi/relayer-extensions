//! Defines the application-specific logic for creating a new balance object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;
use renegade_darkpool_types::{
    balance::{DarkpoolBalanceShare, PostMatchBalanceShare, PreMatchBalanceShare},
    fee::FeeTake,
    settlement_obligation::SettlementObligation,
};
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::{BalanceStateObject, ExpectedStateObject, MasterViewSeed},
};

// ---------
// | Types |
// ---------

/// A transition representing the creation of a new balance object
#[derive(Clone)]
pub struct CreateBalanceTransition {
    /// The recovery ID registered for the balance
    pub recovery_id: Scalar,
    /// The block number in which the recovery ID was registered
    pub block_number: u64,
    /// The data required to create a new balance object
    pub balance_creation_data: BalanceCreationData,
}

/// The data required to create a new balance object
#[derive(Clone)]
pub enum BalanceCreationData {
    /// The balance creation data obtained from deposit of a new balance
    DepositNewBalance {
        /// The full public shares of the new balance
        public_share: DarkpoolBalanceShare,
    },
    /// The balance creation data obtained from the settlement of a public-fill
    /// match into a new output balance
    NewOutputBalanceFromPublicFill {
        /// The public shares of the balance fields unaffected by settlement
        pre_match_balance_share: PreMatchBalanceShare,
        /// The public shares of the balance fields updated by settlement
        post_match_balance_share: PostMatchBalanceShare,
        /// The settlement obligation for the fill
        settlement_obligation: SettlementObligation,
        /// The relayer fee rate used in the fill
        relayer_fee_rate: FixedPoint,
        /// The protocol fee rate used in the fill
        protocol_fee_rate: FixedPoint,
    },
    /// The balance creation data obtained from the settlement of a private-fill
    /// match into a new output balance
    NewOutputBalanceFromPrivateFill {
        /// The public shares of the balance fields unaffected by settlement
        pre_match_balance_share: PreMatchBalanceShare,
        /// The already-updated public shares of the balance fields updated by
        /// settlement
        post_match_balance_share: PostMatchBalanceShare,
    },
}

/// The pre-state required for the creation of a new balance object
struct BalanceCreationPrestate {
    /// The expected state object that will be replaced by the created balance
    expected_state_object: ExpectedStateObject,
    /// The master view seed of the account owning the balance
    master_view_seed: MasterViewSeed,
}

// --------------------------------
// | State Transition Application |
// --------------------------------

impl StateApplicator {
    /// Create a new balance object
    pub async fn create_balance(
        &self,
        transition: CreateBalanceTransition,
    ) -> Result<(), StateTransitionError> {
        let CreateBalanceTransition { recovery_id, block_number, balance_creation_data } =
            transition;

        let BalanceCreationPrestate { expected_state_object, mut master_view_seed } =
            self.get_balance_creation_prestate(recovery_id).await?;

        let recovery_stream_seed = expected_state_object.recovery_stream_seed;

        let balance = construct_new_balance(balance_creation_data, &expected_state_object);

        let next_expected_state_object = master_view_seed.next_expected_state_object();

        let mut conn = self.db_client.get_db_conn().await?;
        conn.transaction(move |conn| {
            async move {
                // Check if the recovery ID has already been processed, no-oping if so
                let recovery_id_processed =
                    self.db_client.check_recovery_id_processed(recovery_id, conn).await?;

                if recovery_id_processed {
                    warn!(
                        "Recovery ID {recovery_id} has already been processed, skipping balance creation"
                    );

                    return Ok(());
                }

                // Mark the recovery ID as processed
                self.db_client.mark_recovery_id_processed(recovery_id, block_number, conn).await?;

                // Check if a balance record already exists for the recovery stream seed.
                // This is possible in the case that we previously processed a metadata update message for the balance.
                let balance_exists = self.db_client.balance_exists(recovery_stream_seed, conn).await?;
                if balance_exists {
                    // We assume the balance details with which the record was originally inserted
                    // match those derived from the public shares
                    warn!("Balance record already exists for recovery stream seed {recovery_stream_seed}, skipping creation");
                    return Ok(());
                }

                // Insert the new balance record
                self.db_client.create_balance(balance, conn).await?;

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

    /// Get the pre-state required for the creation of a new balance object
    async fn get_balance_creation_prestate(
        &self,
        recovery_id: Scalar,
    ) -> Result<BalanceCreationPrestate, StateTransitionError> {
        let mut conn = self.db_client.get_db_conn().await?;
        conn.transaction(|conn| {
            async move {
                let expected_state_object =
                    self.db_client.get_expected_state_object(recovery_id, conn).await?;

                let master_view_seed = self
                    .db_client
                    .get_master_view_seed_by_account_id(expected_state_object.account_id, conn)
                    .await?;

                Ok(BalanceCreationPrestate { expected_state_object, master_view_seed })
            }
            .scope_boxed()
        })
        .await
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Construct a new balance object from the balance creation data
fn construct_new_balance(
    balance_creation_data: BalanceCreationData,
    expected_state_object: &ExpectedStateObject,
) -> BalanceStateObject {
    let ExpectedStateObject { recovery_stream_seed, share_stream_seed, account_id, .. } =
        expected_state_object;

    match balance_creation_data {
        BalanceCreationData::DepositNewBalance { public_share } => BalanceStateObject::new(
            public_share,
            *recovery_stream_seed,
            *share_stream_seed,
            *account_id,
        ),
        BalanceCreationData::NewOutputBalanceFromPublicFill {
            pre_match_balance_share,
            post_match_balance_share,
            settlement_obligation,
            relayer_fee_rate,
            protocol_fee_rate,
        } => {
            // Construct the initial balance from the pre-update public shares
            let public_share = DarkpoolBalanceShare {
                mint: pre_match_balance_share.mint,
                owner: pre_match_balance_share.owner,
                relayer_fee_recipient: pre_match_balance_share.relayer_fee_recipient,
                authority: pre_match_balance_share.authority,
                relayer_fee_balance: post_match_balance_share.relayer_fee_balance,
                protocol_fee_balance: post_match_balance_share.protocol_fee_balance,
                amount: post_match_balance_share.amount,
            };

            let mut balance = BalanceStateObject::new(
                public_share,
                *recovery_stream_seed,
                *share_stream_seed,
                *account_id,
            );

            // Compute the fee take
            let receive_amount = settlement_obligation.amount_out;

            let relayer_fee = scalar_to_u128(&relayer_fee_rate.floor_mul_int(receive_amount));
            let protocol_fee = scalar_to_u128(&protocol_fee_rate.floor_mul_int(receive_amount));

            let fee_take = FeeTake { relayer_fee, protocol_fee };

            balance.balance.apply_obligation_out_balance(&settlement_obligation, &fee_take);

            balance
        },
        BalanceCreationData::NewOutputBalanceFromPrivateFill {
            pre_match_balance_share,
            post_match_balance_share,
        } => {
            // Construct the balance from the updated public shares
            let public_share = DarkpoolBalanceShare {
                mint: pre_match_balance_share.mint,
                owner: pre_match_balance_share.owner,
                relayer_fee_recipient: pre_match_balance_share.relayer_fee_recipient,
                authority: pre_match_balance_share.authority,
                relayer_fee_balance: post_match_balance_share.relayer_fee_balance,
                protocol_fee_balance: post_match_balance_share.protocol_fee_balance,
                amount: post_match_balance_share.amount,
            };

            BalanceStateObject::new(
                public_share,
                *recovery_stream_seed,
                *share_stream_seed,
                *account_id,
            )
        },
    }
}

#[cfg(test)]
mod tests {
    use renegade_darkpool_types::balance::DarkpoolStateBalance;

    use crate::{
        db::test_utils::cleanup_test_db,
        state_transitions::test_utils::{
            gen_deposit_new_balance_transition,
            gen_new_output_balance_from_private_fill_transition,
            gen_new_output_balance_from_public_fill_transition, setup_expected_state_object,
            setup_test_state_applicator, validate_balance_indexing,
            validate_expected_state_object_rotation,
        },
    };

    use super::*;

    /// Index a balance creation and validate the indexing
    async fn validate_balance_creation_indexing(
        test_applicator: &StateApplicator,
        transition: CreateBalanceTransition,
        wrapped_balance: &DarkpoolStateBalance,
        expected_state_object: &ExpectedStateObject,
    ) -> Result<(), StateTransitionError> {
        let db_client = &test_applicator.db_client;
        let recovery_id = transition.recovery_id;

        // Index the balance creation
        test_applicator.create_balance(transition).await?;

        validate_balance_indexing(db_client, wrapped_balance).await?;

        validate_expected_state_object_rotation(db_client, expected_state_object).await?;

        // Assert that the recovery ID is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_recovery_id_processed(recovery_id, &mut conn).await?);

        Ok(())
    }

    /// Test that a new balance deposit is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_deposit_new_balance() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_balance) =
            gen_deposit_new_balance_transition(&expected_state_object);

        validate_balance_creation_indexing(
            &test_applicator,
            transition,
            &wrapped_balance,
            &expected_state_object,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a new output balance creation from a public-fill match
    /// settlement is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_new_output_balance_from_public_fill() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_balance) =
            gen_new_output_balance_from_public_fill_transition(&expected_state_object);

        validate_balance_creation_indexing(
            &test_applicator,
            transition,
            &wrapped_balance,
            &expected_state_object,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }

    /// Test that a new output balance creation from a private-fill match
    /// settlement is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_new_output_balance_from_private_fill() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_balance) =
            gen_new_output_balance_from_private_fill_transition(&expected_state_object);

        validate_balance_creation_indexing(
            &test_applicator,
            transition,
            &wrapped_balance,
            &expected_state_object,
        )
        .await?;

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
