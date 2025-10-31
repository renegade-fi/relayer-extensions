//! Defines the indexer struct, a dependency injection container which stores
//! handles to shared resources

pub mod error;

use crate::{db::client::DbClient, indexer::error::IndexerError};

/// The indexer struct. Stores handles to shared resources.
#[derive(Clone)]
pub struct Indexer {
    /// The database client
    pub db: DbClient,
}

impl Indexer {
    /// Create a new indexer
    pub async fn new(db_url: &str) -> Result<Self, IndexerError> {
        let db = DbClient::new(db_url).await?;

        Ok(Self { db })
    }
}
