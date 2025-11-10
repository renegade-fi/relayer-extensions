//! High-level interface for indexing new master view seeds

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};

use crate::{
    db::{client::DbClient, error::DbError},
    types::MasterViewSeed,
};

impl DbClient {
    /// Index a new account's master view seed, along with the first expected
    /// state object for the account
    pub async fn index_master_view_seed(
        &self,
        mut master_view_seed: MasterViewSeed,
    ) -> Result<(), DbError> {
        let expected_state_object = master_view_seed.next_expected_state_object();

        let mut conn = self.get_db_conn().await?;
        conn.transaction(|conn| {
            async move {
                self.insert_master_view_seed(master_view_seed, conn).await?;
                self.insert_expected_state_object(expected_state_object, conn).await
            }
            .scope_boxed()
        })
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use renegade_circuit_types::csprng::PoseidonCSPRNG;

    use crate::db::test_utils::{
        assert_csprng_state, cleanup_test_db, gen_random_master_view_seed,
        get_expected_object_nullifier, setup_test_db_client,
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
            .get_account_master_view_seed(expected_master_view_seed.account_id, &mut conn)
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
        let first_nullifier =
            get_expected_object_nullifier(master_view_seed, 0 /* object number */);

        let indexed_expected_state_object = db_client
            .get_expected_state_object(first_nullifier, &mut conn)
            .await?
            .ok_or(DbError::custom("Expected state object not found"))?;

        // The first expected state object for the account should be using the first
        // recovery & share stream seeds sampled from the master view seed.
        let expected_recovery_stream =
            PoseidonCSPRNG::new(master_view_seed.recovery_seed_csprng.get_ith(0));

        let indexed_recovery_stream = &indexed_expected_state_object.recovery_stream;
        assert_csprng_state(
            indexed_recovery_stream,
            expected_recovery_stream.seed,
            0, // expected_index
        );

        let expected_share_stream =
            PoseidonCSPRNG::new(master_view_seed.share_seed_csprng.get_ith(0));

        let indexed_share_stream = &indexed_expected_state_object.share_stream;
        assert_csprng_state(
            indexed_share_stream,
            expected_share_stream.seed,
            0, // expected_index
        );

        Ok(())
    }

    /// Test that a new master view seed is indexed correctly.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_index_master_view_seed() -> Result<(), DbError> {
        let test_db_client = setup_test_db_client().await?;
        let db_client = test_db_client.get_client();

        let master_view_seed = gen_random_master_view_seed();

        // Index the master view seed
        db_client.index_master_view_seed(master_view_seed.clone()).await?;

        validate_new_master_view_seed_indexing(db_client, &master_view_seed).await?;

        validate_new_expected_state_object_indexing(db_client, &master_view_seed).await?;

        cleanup_test_db(test_db_client).await
    }
}
