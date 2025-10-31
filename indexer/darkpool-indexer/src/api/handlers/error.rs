//! Handler error definitions

use crate::db::error::DbError;

/// Handler errors
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// An error in the database client
    #[error("database error: {0}")]
    Db(#[from] DbError),
}
