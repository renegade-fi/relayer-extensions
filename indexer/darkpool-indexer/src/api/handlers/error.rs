//! Handler error definitions

use aws_sdk_sqs::error::SdkError;

use crate::state_transitions::error::StateTransitionError;

/// Handler errors
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// An error in the state transition applicator
    #[error("state transition error: {0}")]
    StateTransition(#[from] StateTransitionError),
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
