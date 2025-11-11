//! High-level interface for indexing nullifier spends

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use renegade_circuit_types::{balance::Balance, intent::Intent};
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
                    self.handle_existing_object_nullifier_spend(nullifier_spend_data, conn).await?;
                }

                // Mark the nullifier as processed
                self.mark_nullifier_processed(nullifier, block_number, conn).await
            }
            .scope_boxed()
        })
        .await?;

        Ok(())
    }

    /// Handle the spending of an existing state object's nullifier
    async fn handle_existing_object_nullifier_spend(
        &self,
        nullifier_spend_data: NullifierSpendData,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let NullifierSpendData { nullifier, updated_public_shares, updated_shares_index, .. } =
            nullifier_spend_data;

        // Update the public & private shares of the associated generic state object
        let mut generic_state_object =
            self.get_generic_state_object_for_nullifier(nullifier, conn).await?;

        generic_state_object.update(&updated_public_shares, updated_shares_index);
        self.update_generic_state_object(generic_state_object.clone(), conn).await?;

        // Update the associated typed state object
        match generic_state_object.object_type {
            StateObjectType::Intent => {
                self.update_intent_state_object(&generic_state_object, conn).await
            },
            StateObjectType::Balance => {
                self.update_balance_state_object(&generic_state_object, conn).await
            },
        }
    }

    /// Handle the spending of a state object's first nullifier
    async fn handle_first_object_nullifier_spend(
        &self,
        nullifier_spend_data: NullifierSpendData,
        expected_state_object: ExpectedStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let NullifierSpendData { updated_public_shares, state_object_type, .. } =
            nullifier_spend_data;

        // Create the generic state object
        let generic_state_object = GenericStateObject::new(
            expected_state_object.recovery_stream.seed,
            expected_state_object.account_id,
            state_object_type,
            expected_state_object.share_stream.seed,
            expected_state_object.owner_address,
            updated_public_shares,
        );

        self.create_generic_state_object(generic_state_object.clone(), conn).await?;

        // Create the appropriate typed state object
        match generic_state_object.object_type {
            StateObjectType::Intent => {
                self.create_intent_state_object(&generic_state_object, conn).await?
            },
            StateObjectType::Balance => {
                self.create_balance_state_object(&generic_state_object, conn).await?
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
        generic_state_object: &GenericStateObject,
        conn: &mut DbConn,
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

        let intent = generic_state_object.reconstruct_circuit_type::<Intent>().inner;

        let intent_state_object = IntentStateObject::new(
            intent,
            generic_state_object.recovery_stream.seed,
            generic_state_object.account_id,
        );

        self.create_intent(intent_state_object, conn).await?;

        Ok(())
    }

    /// Update an existing intent state object from the given generic state
    /// object
    async fn update_intent_state_object(
        &self,
        generic_state_object: &GenericStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let wrapped_intent = generic_state_object.reconstruct_circuit_type::<Intent>();

        self.update_intent_core(wrapped_intent.recovery_stream.seed, wrapped_intent.inner, conn)
            .await
    }

    /// Create a balance state object from a newly-created generic state object
    async fn create_balance_state_object(
        &self,
        generic_state_object: &GenericStateObject,
        conn: &mut DbConn,
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

        let balance = generic_state_object.reconstruct_circuit_type::<Balance>().inner;

        let balance_state_object = BalanceStateObject::new(
            balance,
            generic_state_object.recovery_stream.seed,
            generic_state_object.account_id,
        );

        self.create_balance(balance_state_object, conn).await?;

        Ok(())
    }

    /// Update an existing balance state object from the given generic state
    /// object
    async fn update_balance_state_object(
        &self,
        generic_state_object: &GenericStateObject,
        conn: &mut DbConn,
    ) -> Result<(), DbError> {
        let wrapped_balance = generic_state_object.reconstruct_circuit_type::<Balance>();

        self.update_balance_core(wrapped_balance.recovery_stream.seed, wrapped_balance.inner, conn)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;

    use alloy::primitives::Address;
    use rand::{Rng, thread_rng};
    use renegade_circuit_types::{
        Amount,
        balance::{Balance, BalanceShare},
        state_wrapper::StateWrapper,
        traits::{BaseType, CircuitBaseType, SecretShareBaseType, SecretShareType},
    };

    use crate::db::test_utils::{
        assert_csprng_state, cleanup_test_db, gen_random_master_view_seed, setup_test_db_client,
    };

    use super::*;

    /// Sets up an expected state object in the DB, generating a new master view
    /// seed for the account owning the state object.
    ///
    /// Returns the expected state object.
    async fn setup_expected_state_object(
        db_client: &DbClient,
    ) -> Result<ExpectedStateObject, DbError> {
        let mut master_view_seed = gen_random_master_view_seed();
        db_client.index_master_view_seed(master_view_seed.clone()).await?;
        Ok(master_view_seed.next_expected_state_object())
    }

    /// Generate the nullifier spend data which should result in the given
    /// expected state object being indexed as a new balance.
    ///
    /// Returns the nullifier spend data, along with the expected balance
    /// object.
    fn gen_new_balance_nullifier_spend_data(
        expected_state_object: &ExpectedStateObject,
    ) -> (NullifierSpendData, StateWrapper<Balance>) {
        let mint = Address::random();
        let relayer_fee_recipient = Address::random();
        let one_time_authority = Address::random();

        let balance = Balance::new(
            mint,
            expected_state_object.owner_address,
            relayer_fee_recipient,
            one_time_authority,
        );

        let mut wrapped_balance = StateWrapper::new(
            balance,
            expected_state_object.share_stream.seed,
            expected_state_object.recovery_stream.seed,
        );

        // New state objects start at version 1 (after their 0th nullifier has been
        // spent).
        // This means their initial recovery stream index is 2 (version = index - 1).
        wrapped_balance.recovery_stream.advance_by(2);

        let updated_public_shares = wrapped_balance.public_share().to_scalars();

        let nullifier_spend_data = NullifierSpendData {
            nullifier: expected_state_object.nullifier,
            block_number: 0,
            state_object_type: StateObjectType::Balance,
            updated_public_shares,
            updated_shares_index: 0,
        };

        (nullifier_spend_data, wrapped_balance)
    }

    /// Generate the nullifier spend data which should result in the given
    /// balance being updated with a deposit.
    ///
    /// Returns the nullifier spend data, along with the updated balance.
    fn gen_deposit_nullifier_spend_data(
        initial_balance: &StateWrapper<Balance>,
    ) -> (NullifierSpendData, StateWrapper<Balance>) {
        let spent_nullifier = initial_balance.compute_nullifier();

        let mut updated_balance = initial_balance.clone();

        // Advance the recovery stream to indicate the next object version
        updated_balance.recovery_stream.advance_by(1);

        // Apply a random deposit amount to the balance
        let deposit_amount: Amount = thread_rng().r#gen();
        updated_balance.inner.amount += deposit_amount;

        // We re-encrypt only the updated shares of the balance, which in this case
        // pertain only to the amount
        let updated_balance_amount = updated_balance.inner.amount;
        let updated_public_shares =
            updated_balance.stream_cipher_encrypt(&updated_balance_amount).to_scalars();

        // The balance amount is the last field in the secret-sharing of the balance.
        // We compute the updated shares index accordingly.
        let updated_shares_index = Balance::NUM_SCALARS - Amount::NUM_SCALARS;

        // We write the updated public shares to the appropriate slice of the total
        // public sharing of the balance
        let mut all_public_shares = updated_balance.public_share().to_scalars();
        all_public_shares[updated_shares_index..].copy_from_slice(&updated_public_shares);
        updated_balance.public_share =
            BalanceShare::from_scalars(&mut all_public_shares.into_iter());

        // Construct the associated nullifier spend data
        let nullifier_spend_data = NullifierSpendData {
            nullifier: spent_nullifier,
            block_number: 0,
            state_object_type: StateObjectType::Balance,
            updated_public_shares,
            updated_shares_index,
        };

        (nullifier_spend_data, updated_balance)
    }

    /// Validate the indexing of a generic state object against the expected
    /// circuit type
    async fn validate_generic_state_object_indexing<T>(
        db_client: &DbClient,
        expected_reconstructed_object: &StateWrapper<T>,
    ) -> Result<(), DbError>
    where
        T: SecretShareBaseType + CircuitBaseType + Debug + Eq,
        T::ShareType: CircuitBaseType,
        <T::ShareType as SecretShareType>::Base: Into<T>,
    {
        let mut conn = db_client.get_db_conn().await?;

        let nullifier = expected_reconstructed_object.compute_nullifier();
        let indexed_generic_state_object =
            db_client.get_generic_state_object_for_nullifier(nullifier, &mut conn).await?;

        // Assert that the indexed generic state object's CSPRNG states are correctly
        // advanced
        let indexed_recovery_stream = &indexed_generic_state_object.recovery_stream;
        let expected_recovery_stream = &expected_reconstructed_object.recovery_stream;
        assert_csprng_state(
            indexed_recovery_stream,
            expected_recovery_stream.seed,
            expected_recovery_stream.index,
        );

        let indexed_share_stream = &indexed_generic_state_object.share_stream;
        let expected_share_stream = &expected_reconstructed_object.share_stream;
        assert_csprng_state(
            indexed_share_stream,
            expected_share_stream.seed,
            expected_share_stream.index,
        );

        // Assert that the indexed generic state object's shares properly reconstruct
        // the expected object
        let public_share = T::ShareType::from_scalars(
            &mut indexed_generic_state_object.public_shares.clone().into_iter(),
        );

        let private_share = T::ShareType::from_scalars(
            &mut indexed_generic_state_object.private_shares.clone().into_iter(),
        );

        let reconstructed_object: T = public_share.add_shares(&private_share).into();

        assert_eq!(reconstructed_object, expected_reconstructed_object.inner);

        Ok(())
    }

    /// Validate the rotation of an account's next expected state object
    async fn validate_expected_state_object_rotation(
        db_client: &DbClient,
        old_expected_state_object: &ExpectedStateObject,
    ) -> Result<(), DbError> {
        let mut conn = db_client.get_db_conn().await?;

        // Assert that the indexed master view seed's CSPRNG states are advanced
        // correctly
        let indexed_master_view_seed = db_client
            .get_account_master_view_seed(old_expected_state_object.account_id, &mut conn)
            .await?;

        let recovery_seed_stream = &indexed_master_view_seed.recovery_seed_csprng;
        assert_csprng_state(recovery_seed_stream, recovery_seed_stream.seed, 2);

        let share_seed_stream = &indexed_master_view_seed.share_seed_csprng;
        assert_csprng_state(share_seed_stream, share_seed_stream.seed, 2);

        // Assert that the next expected state object is indexed correctly
        let expected_recovery_stream_seed = recovery_seed_stream.get_ith(1);
        let expected_share_stream_seed = share_seed_stream.get_ith(1);
        let next_expected_state_object = ExpectedStateObject::new(
            indexed_master_view_seed.account_id,
            indexed_master_view_seed.owner_address,
            expected_recovery_stream_seed,
            expected_share_stream_seed,
        );

        let indexed_next_expected_state_object = db_client
            .get_expected_state_object(next_expected_state_object.nullifier, &mut conn)
            .await?
            .ok_or(DbError::custom("Next expected state object not found"))?;

        assert_eq!(indexed_next_expected_state_object, next_expected_state_object);

        // Assert that the old expected state object is deleted
        let maybe_deleted_expected_state_object = db_client
            .get_expected_state_object(old_expected_state_object.nullifier, &mut conn)
            .await?;

        assert!(maybe_deleted_expected_state_object.is_none());

        Ok(())
    }

    /// Test that a state object's first nullifier spend is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_new_balance_nullifier_spend() -> Result<(), DbError> {
        let test_db_client = setup_test_db_client().await?;
        let db_client = test_db_client.get_client();
        let mut conn = db_client.get_db_conn().await?;

        let expected_state_object = setup_expected_state_object(db_client).await?;
        let (nullifier_spend_data, wrapped_balance) =
            gen_new_balance_nullifier_spend_data(&expected_state_object);

        // Index the new balance's nullifier spend
        db_client.index_nullifier_spend(nullifier_spend_data.clone()).await?;

        validate_generic_state_object_indexing(db_client, &wrapped_balance).await?;

        // Assert that the indexed balance object matches the expected balance object
        let indexed_balance_object =
            db_client.get_balance(expected_state_object.recovery_stream.seed, &mut conn).await?;

        assert_eq!(indexed_balance_object.balance, wrapped_balance.inner);

        validate_expected_state_object_rotation(db_client, &expected_state_object).await?;

        // Assert that the nullifier is marked as processed
        assert!(db_client.nullifier_processed(nullifier_spend_data.nullifier, &mut conn).await?);

        cleanup_test_db(test_db_client).await
    }

    /// Test that a nullifier spend of an existing state object is indexed
    /// correctly
    #[tokio::test(flavor = "multi_thread")]
    async fn test_existing_balance_nullifier_spend() -> Result<(), DbError> {
        let test_db_client = setup_test_db_client().await?;
        let db_client = test_db_client.get_client();
        let mut conn = db_client.get_db_conn().await?;

        let expected_state_object = setup_expected_state_object(db_client).await?;
        let (initial_nullifier_spend_data, initial_wrapped_balance) =
            gen_new_balance_nullifier_spend_data(&expected_state_object);

        // Index the new balance's nullifier spend
        db_client.index_nullifier_spend(initial_nullifier_spend_data.clone()).await?;

        // Generate subsequent nullifier spend data
        let (deposit_nullifier_spend_data, updated_wrapped_balance) =
            gen_deposit_nullifier_spend_data(&initial_wrapped_balance);

        // Index subsequent nullifier spend
        db_client.index_nullifier_spend(deposit_nullifier_spend_data.clone()).await?;

        validate_generic_state_object_indexing(db_client, &updated_wrapped_balance).await?;

        // Assert that the indexed balance object matches the updated balance
        let indexed_balance_object =
            db_client.get_balance(updated_wrapped_balance.recovery_stream.seed, &mut conn).await?;

        assert_eq!(indexed_balance_object.balance, updated_wrapped_balance.inner);

        // Assert that the nullifier is marked as processed
        assert!(
            db_client
                .nullifier_processed(deposit_nullifier_spend_data.nullifier, &mut conn)
                .await?
        );

        cleanup_test_db(test_db_client).await
    }
}
