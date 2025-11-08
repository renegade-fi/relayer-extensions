//! High-level interface for indexing nullifier spends

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::{
    balance::BalanceShare,
    intent::IntentShare,
    traits::{BaseType, SecretShareType},
};
use renegade_constants::Scalar;
use tracing::{info, warn};

use crate::{
    db::{
        client::{DbClient, DbConn},
        error::DbError,
    },
    types::{
        BalanceStateObject, ExpectedStateObject, GenericStateObject, IntentStateObject,
        NullifierSpendData, StateObjectType,
    },
};

impl DbClient {
    /// Index a nullifier spend
    pub async fn index_nullifier_spend(
        &self,
        nullifier_spend_data: NullifierSpendData,
    ) -> Result<(), DbError> {
        let mut conn = self.get_db_conn().await?;
        conn.transaction(|conn| {
            async move {
                // Extract the nullifier and block number from the data before we move it
                let nullifier = nullifier_spend_data.nullifier;
                let block_number = nullifier_spend_data.block_number;

                // Check if the nullifier has already been processed
                let nullifier_processed = self.nullifier_processed(nullifier, conn).await?;
                if nullifier_processed {
                    warn!("Nullifier {} has already been processed", nullifier);
                    return Ok(());
                }

                // Check if this is the nullifier for an expected state object
                let maybe_expected_state_object =
                    self.get_expected_state_object(nullifier, conn).await?;

                if let Some(expected_state_object) = maybe_expected_state_object {
                    self.handle_first_object_nullifier_spend(
                        nullifier_spend_data,
                        expected_state_object,
                        conn,
                    )
                    .await?;
                } else {
                    // TODO: Handle nullifier spend messages for existing state
                    // objects
                }

                // Mark the nullifier as processed
                self.mark_nullifier_processed(nullifier, block_number, conn).await
            }
            .scope_boxed()
        })
        .await?;

        Ok(())
    }

    /// Handle the spending of a state object's first nullifier
    async fn handle_first_object_nullifier_spend(
        &self,
        nullifier_spend_data: NullifierSpendData,
        expected_state_object: ExpectedStateObject,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        let NullifierSpendData { updated_public_shares, state_object_type, .. } =
            nullifier_spend_data;

        // Generate the private shares for the state object
        let private_shares: Vec<Scalar> =
            expected_state_object.share_stream.clone().take(updated_public_shares.len()).collect();

        // Create the generic state object
        let generic_state_object = GenericStateObject::new(
            expected_state_object.recovery_stream.seed,
            expected_state_object.account_id,
            state_object_type,
            expected_state_object.share_stream.seed,
            expected_state_object.owner_address,
            updated_public_shares,
            private_shares,
        );

        self.create_generic_state_object(generic_state_object.clone(), conn).await?;

        // Create the appropriate typed state object
        match generic_state_object.object_type {
            StateObjectType::Intent => {
                self.create_intent_state_object(generic_state_object, conn).await?
            },
            StateObjectType::Balance => {
                self.create_balance_state_object(generic_state_object, conn).await?
            },
        };

        // Delete the expected state object record for the now-indexed state object
        self.delete_expected_state_object(expected_state_object.nullifier, conn).await?;

        // Create an expected state object record for the next state object for the
        // account, updating the master view seed CSPRNG states in the process
        let mut master_view_seed =
            self.get_account_master_view_seed(expected_state_object.account_id, conn).await?;

        let next_expected_state_object = master_view_seed.next_expected_state_object();
        self.insert_expected_state_object(next_expected_state_object, conn).await?;
        self.update_master_view_seed(master_view_seed, conn).await?;

        Ok(())
    }

    /// Create an intent state object from a newly-created generic state object
    async fn create_intent_state_object(
        &self,
        generic_state_object: GenericStateObject,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        // First, check if the associated intent object already exists in the DB.
        // This is possible in the case that we previously processed a metadata update
        // message for it.
        let intent_exists =
            self.intent_exists(generic_state_object.recovery_stream.seed, conn).await?;

        if intent_exists {
            // We assume that the intent details with which the record was originally
            // created match those derived from the newly-created generic state
            // object
            info!("Intent object record already exists, skipping creation");
            return Ok(());
        }

        let intent_public_share =
            IntentShare::from_scalars(&mut generic_state_object.public_shares.into_iter());

        let intent_private_share =
            IntentShare::from_scalars(&mut generic_state_object.private_shares.into_iter());

        let intent = intent_public_share.add_shares(&intent_private_share);

        let intent_state_object = IntentStateObject::new(
            intent,
            generic_state_object.recovery_stream.seed,
            generic_state_object.account_id,
        );

        self.create_intent(intent_state_object, conn).await?;

        Ok(())
    }

    /// Create a balance state object from a newly-created generic state object
    async fn create_balance_state_object(
        &self,
        generic_state_object: GenericStateObject,
        conn: &mut DbConn<'_>,
    ) -> Result<(), DbError> {
        // First, check if the associated balance object already exists in the DB.
        // This is possible in the case that we previously processed a metadata update
        // message for it.
        let balance_exists =
            self.balance_exists(generic_state_object.recovery_stream.seed, conn).await?;

        if balance_exists {
            // We assume that the balance details with which the record was originally
            // created match those derived from the newly-created generic state
            // object
            info!("Balance object record already exists, skipping creation");
            return Ok(());
        }

        let balance_public_share =
            BalanceShare::from_scalars(&mut generic_state_object.public_shares.into_iter());

        let balance_private_share =
            BalanceShare::from_scalars(&mut generic_state_object.private_shares.into_iter());

        let balance = balance_public_share.add_shares(&balance_private_share);

        let balance_state_object = BalanceStateObject::new(
            balance,
            generic_state_object.recovery_stream.seed,
            generic_state_object.account_id,
        );

        self.create_balance(balance_state_object, conn).await?;

        Ok(())
    }
}
