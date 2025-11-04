//! Handler logic SQS messages polled by the darkpool indexer

use darkpool_indexer_api::types::sqs::MasterViewSeedMessage;
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};

use crate::{
    api::handlers::error::HandlerError,
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
) -> Result<(), HandlerError> {
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
