//! State transition error definitions

use crate::db::error::DbError;

/// State transition errors
#[derive(Debug, thiserror::Error)]
pub enum StateTransitionError {
    /// An error in the database client
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

impl From<diesel::result::Error> for StateTransitionError {
    fn from(error: diesel::result::Error) -> Self {
        StateTransitionError::Db(DbError::from(error))
    }
}
