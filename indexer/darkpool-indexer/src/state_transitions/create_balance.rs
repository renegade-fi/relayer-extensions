//! Defines the application-specific logic for creating a new balance object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::balance::BalanceShare;
use renegade_constants::Scalar;
use tracing::warn;

use crate::{
    state_transitions::{
        StateApplicator, error::StateTransitionError,
    },
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
    /// The public shares of the balance
    pub public_share: BalanceShare,
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
        let CreateBalanceTransition { recovery_id, block_number, public_share } = transition;

        let BalanceCreationPrestate { expected_state_object, mut master_view_seed } = self.get_balance_creation_prestate(recovery_id).await?;

        let ExpectedStateObject {
            recovery_id,
            recovery_stream_seed,
            share_stream_seed,
            account_id,
        } = expected_state_object;

        let balance = BalanceStateObject::new(
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
    async fn get_balance_creation_prestate(&self, recovery_id: Scalar) -> Result<BalanceCreationPrestate, StateTransitionError> {
        let mut conn = self.db_client.get_db_conn().await?;
        conn.transaction(|conn| {
            async move {
                let expected_state_object =
                    self.db_client.get_expected_state_object(recovery_id, conn).await?;

                let master_view_seed = self.db_client.get_master_view_seed_by_account_id(expected_state_object.account_id, conn).await?;

                Ok(BalanceCreationPrestate { expected_state_object, master_view_seed })
            }.scope_boxed()
        }).await
    }
}

#[cfg(test)]
mod tests {
    use crate::{db::test_utils::cleanup_test_db, state_transitions::test_utils::{gen_create_balance_transition, setup_expected_state_object, setup_test_state_applicator, validate_balance_indexing, validate_expected_state_object_rotation}};

    use super::*;

    /// Test that a balance creation is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_balance() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;
        let db_client = &test_applicator.db_client;

        let expected_state_object = setup_expected_state_object(&test_applicator).await?;
        let (transition, wrapped_balance) =
            gen_create_balance_transition(&expected_state_object);

        // Index the balance creation
        test_applicator.create_balance(transition.clone()).await?;

        validate_balance_indexing(db_client, &wrapped_balance).await?;

        validate_expected_state_object_rotation(db_client, &expected_state_object).await?;

        // Assert that the recovery ID is marked as processed
        let mut conn = db_client.get_db_conn().await?;
        assert!(db_client.check_recovery_id_processed(transition.recovery_id, &mut conn).await?);

        cleanup_test_db(&postgres).await?;

        Ok(())
    }
}
