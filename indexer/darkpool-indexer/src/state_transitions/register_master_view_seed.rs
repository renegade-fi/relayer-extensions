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
