//! API error definitions

use crate::db::error::DbError;

/// API errors
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// An error handling an SQS message
    #[error("SQS message handling error: {0}")]
    Db(#[from] DbError),
}
