//! Defines the application-specific logic for registering a new master view
//! seed

use darkpool_indexer_api::types::sqs::MasterViewSeedMessage;
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use tracing::warn;

use crate::{
    state_transitions::{StateApplicator, error::StateTransitionError},
    types::MasterViewSeed,
};

impl StateApplicator {
    /// Register a new master view seed
    pub async fn register_master_view_seed(
        &self,
        transition: MasterViewSeedMessage,
    ) -> Result<(), StateTransitionError> {
        let MasterViewSeedMessage { account_id, owner_address, seed } = transition;
        let mut master_view_seed = MasterViewSeed::new(account_id, owner_address, seed);

        let expected_state_object = master_view_seed.next_expected_state_object();

        let mut conn = self.db_client.get_db_conn().await?;
        conn.transaction(move |conn| {
            async move {
                let master_view_seed_exists = self.db_client.master_view_seed_exists(account_id, conn).await?;
                if master_view_seed_exists {
                    warn!("Master view seed already exists for account {account_id}, skipping registration");
                    return Ok(());
                }

                self.db_client.insert_master_view_seed(master_view_seed, conn).await?;
                self.db_client.insert_expected_state_object(expected_state_object, conn).await?;
                Ok(())
            }.scope_boxed()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        db::{client::DbClient, error::DbError, test_utils::cleanup_test_db},
        state_transitions::test_utils::{
            assert_csprng_state, gen_random_master_view_seed, get_expected_object_recovery_id,
            setup_test_state_applicator,
        },
    };

    use super::*;

    /// Validate the indexing of a new master view seed
    async fn validate_new_master_view_seed_indexing(
        db_client: &DbClient,
        expected_master_view_seed: &MasterViewSeed,
    ) -> Result<(), DbError> {
        // Retrieve the indexed master view seed
        let mut conn = db_client.get_db_conn().await?;
        let indexed_master_view_seed = db_client
            .get_master_view_seed_by_account_id(expected_master_view_seed.account_id, &mut conn)
            .await?;

        // The master view seed should have been indexed with its recovery & share seed
        // CSPRNGs advanced by 1, as it was already used to generate the first
        // expected state object.
        let indexed_recovery_seed_stream = &indexed_master_view_seed.recovery_seed_csprng;
        assert_csprng_state(
            indexed_recovery_seed_stream,
            expected_master_view_seed.recovery_seed_csprng.seed,
            1, // expected_index
        );

        let indexed_share_seed_stream = &indexed_master_view_seed.share_seed_csprng;
        assert_csprng_state(
            indexed_share_seed_stream,
            expected_master_view_seed.share_seed_csprng.seed,
            1, // expected_index
        );

        Ok(())
    }

    /// Validate the indexing of a new expected state object
    async fn validate_new_expected_state_object_indexing(
        db_client: &DbClient,
        master_view_seed: &MasterViewSeed,
    ) -> Result<(), DbError> {
        let mut conn = db_client.get_db_conn().await?;

        // Retrieve the first expected state object for the account
        let first_recovery_id =
            get_expected_object_recovery_id(master_view_seed, 0 /* object number */);

        let indexed_expected_state_object =
            db_client.get_expected_state_object(first_recovery_id, &mut conn).await?;

        // The first expected state object for the account should be using the first
        // recovery & share stream seeds sampled from the master view seed.
        let expected_recovery_stream_seed = master_view_seed.recovery_seed_csprng.get_ith(0);

        let indexed_recovery_stream_seed = indexed_expected_state_object.recovery_stream_seed;
        assert_eq!(indexed_recovery_stream_seed, expected_recovery_stream_seed);

        let expected_share_stream_seed = master_view_seed.share_seed_csprng.get_ith(0);

        let indexed_share_stream_seed = indexed_expected_state_object.share_stream_seed;
        assert_eq!(indexed_share_stream_seed, expected_share_stream_seed);

        Ok(())
    }

    /// Test that the registration of a new master view seed is indexed
    /// correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_register_master_view_seed() -> Result<(), StateTransitionError> {
        let (test_applicator, postgres) = setup_test_state_applicator().await?;

        let master_view_seed = gen_random_master_view_seed();
        let master_view_seed_message = MasterViewSeedMessage {
            account_id: master_view_seed.account_id,
            owner_address: master_view_seed.owner_address,
            seed: master_view_seed.seed,
        };

        // Index the master view seed
        test_applicator.register_master_view_seed(master_view_seed_message).await?;

        validate_new_master_view_seed_indexing(&test_applicator.db_client, &master_view_seed)
            .await?;

        validate_new_expected_state_object_indexing(&test_applicator.db_client, &master_view_seed)
            .await?;

        cleanup_test_db(postgres).await?;

        Ok(())
    }
}
