//! Handler error definitions

use aws_sdk_sqs::error::SdkError;

use crate::db::error::DbError;

/// Handler errors
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// An error in the database client
    #[error("database error: {0}")]
    Db(#[from] DbError),
    /// An error with AWS SQS
    #[error("SQS error: {0}")]
    Sqs(String),
    /// An error with the RPC client
    #[error("RPC client error: {0}")]
    Rpc(String),
    /// An error parsing a value
    #[error("parse error: {0}")]
    Parse(String),
    /// An error de/serializing a value
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[allow(clippy::needless_pass_by_value)]
impl HandlerError {
    /// Create a new parse error
    pub fn parse<T: ToString>(msg: T) -> Self {
        Self::Parse(msg.to_string())
    }

    /// Create a new RPC error
    pub fn rpc<T: ToString>(msg: T) -> Self {
        Self::Rpc(msg.to_string())
    }
}

impl<E, R> From<SdkError<E, R>> for HandlerError {
    fn from(value: SdkError<E, R>) -> Self {
        HandlerError::Sqs(value.to_string())
    }
}
