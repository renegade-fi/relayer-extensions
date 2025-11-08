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
