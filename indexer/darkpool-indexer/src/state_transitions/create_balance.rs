//! Defines the application-specific logic for creating a new balance object.

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_constants::Scalar;
use tracing::{info, warn};

use crate::{
    state_transitions::{
        StateApplicator, error::StateTransitionError, types::CreateBalanceTransition,
    },
    types::{BalanceStateObject, ExpectedStateObject, MasterViewSeed},
};

// ---------
// | Types |
// ---------

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

                // Check if a balance record already exists for the recovery stream seed.
                // This is possible in the case that we previously processed a metadata update message for the balance.
                let balance_exists = self.db_client.balance_exists(recovery_stream_seed, conn).await?;
                if balance_exists {
                    // We assume the balance details with which the record was originally inserted
                    // match those derived from the public shares
                    info!("Balance record already exists for recovery stream seed {recovery_stream_seed}, skipping creation");
                    return Ok(());
                }

                // Insert the new balance record
                self.db_client.create_balance(balance, conn).await?;

                // Delete the now-created expected state object, insert the next one,
                // & update the master view seed as its CSPRNG states have advanced
                self.db_client.delete_expected_state_object(recovery_id, conn).await?;
                self.db_client.insert_expected_state_object(next_expected_state_object, conn).await?;
                self.db_client.update_master_view_seed(master_view_seed, conn).await?;

                // Mark the recovery ID as processed
                self.db_client.mark_recovery_id_processed(recovery_id, block_number, conn).await?;

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

                let master_view_seed = self.db_client.get_account_master_view_seed(expected_state_object.account_id, conn).await?;

                Ok(BalanceCreationPrestate { expected_state_object, master_view_seed })
            }.scope_boxed()
        }).await
    }
}
