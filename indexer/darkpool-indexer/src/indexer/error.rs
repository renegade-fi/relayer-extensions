//! Top-level indexer error definitions

use crate::{api::handlers::error::HandlerError, db::error::DbError};

/// Indexer errors
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// An error in the database client
    #[error("database error: {0}")]
    Db(#[from] DbError),
    /// An error in an API handler
    #[error("API handler error: {0}")]
    ApiHandler(#[from] HandlerError),
}
