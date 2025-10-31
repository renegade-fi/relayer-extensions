//! Handler logic SQS messages polled by the darkpool indexer

use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};

use crate::{
    api::{error::ApiError, types::sqs::MasterViewSeedMessage},
    crypto_mocks::{
        encryption_stream::sample_encryption_seed,
        identifier_stream::{sample_identifier_seed, sample_nullifier},
    },
    indexer::Indexer,
};

// -------------
// | Constants |
// -------------

/// The index of the identifier stream seed for an account's first state object
const FIRST_OBJECT_IDX: usize = 0;

/// A state object's first version
const OBJECT_FIRST_VERSION: usize = 0;

// ------------
// | Handlers |
// ------------

/// Handle a SQS message representing the registration of a new master view seed
pub async fn handle_master_view_seed_message(
    message: MasterViewSeedMessage,
    indexer: &Indexer,
) -> Result<(), ApiError> {
    let MasterViewSeedMessage { account_id, owner_address, seed } = message;

    let first_object_identifier_seed = sample_identifier_seed(seed, FIRST_OBJECT_IDX);
    let first_nullifier = sample_nullifier(first_object_identifier_seed, OBJECT_FIRST_VERSION);

    let first_object_encryption_seed = sample_encryption_seed(seed, FIRST_OBJECT_IDX);

    let mut conn = indexer.db.get_db_conn().await?;
    conn.transaction(|conn| {
        async move {
            indexer
                .db
                .insert_master_view_seed(account_id, owner_address.clone(), seed, conn)
                .await?;

            indexer
                .db
                .insert_expected_nullifier(
                    first_nullifier,
                    account_id,
                    owner_address,
                    first_object_identifier_seed,
                    first_object_encryption_seed,
                    conn,
                )
                .await
        }
        .scope_boxed()
    })
    .await?;

    Ok(())
}

/// SQS handler test suite
#[cfg(test)]
mod test {
    use rand::thread_rng;
    use renegade_constants::Scalar;
    use uuid::Uuid;

    use crate::indexer::error::IndexerError;

    use super::*;

    /// Build an indexer configured with a database URL from the environment
    async fn build_test_indexer() -> Result<Indexer, IndexerError> {
        let db_url = env!("DATABASE_URL");
        Indexer::new(db_url).await
    }

    /// Generate a random master view seed message
    fn gen_rand_master_view_seed_msg() -> MasterViewSeedMessage {
        MasterViewSeedMessage {
            account_id: Uuid::new_v4(),
            owner_address: "0x1234567890".to_string(),
            seed: Scalar::random(&mut thread_rng()),
        }
    }

    /// Compute the first nullifier for a given master view seed
    fn compute_first_nullifier(master_view_seed: Scalar) -> Scalar {
        let first_object_identifier_seed =
            sample_identifier_seed(master_view_seed, FIRST_OBJECT_IDX);

        sample_nullifier(first_object_identifier_seed, OBJECT_FIRST_VERSION)
    }

    #[tokio::test]
    async fn test_handle_master_view_seed_message() -> Result<(), IndexerError> {
        let indexer = build_test_indexer().await?;

        // Generate a random master view seed message
        let message = gen_rand_master_view_seed_msg();
        let account_id = message.account_id;
        let master_view_seed = message.seed;

        // Invoke the handler
        handle_master_view_seed_message(message, &indexer).await?;

        let mut conn = indexer.db.get_db_conn().await?;

        // Check that the master view seed record was inserted
        let master_view_seed_record =
            indexer.db.get_account_master_view_seed(account_id, &mut conn).await;

        assert!(master_view_seed_record.is_ok());

        // Check tahat the expected nullifier record was inserted
        let first_nullifier = compute_first_nullifier(master_view_seed);
        let expected_nullifier =
            indexer.db.get_expected_nullifier(first_nullifier, &mut conn).await;

        assert!(expected_nullifier.is_ok());

        Ok(())
    }
}
