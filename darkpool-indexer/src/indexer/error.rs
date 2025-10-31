//! Top-level indexer error definitions

use crate::{api::error::ApiError, db::error::DbError};

/// Indexer errors
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// An error in the database client
    #[error("database error: {0}")]
    Db(#[from] DbError),
    /// An error in the API
    #[error("API error: {0}")]
    Api(#[from] ApiError),
}
