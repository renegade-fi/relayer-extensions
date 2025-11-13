//! Handler error definitions

use aws_sdk_sqs::error::SdkError;

use crate::{
    darkpool_client::error::DarkpoolClientError, state_transitions::error::StateTransitionError,
};

/// Handler errors
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// An error in the state transition applicator
    #[error("state transition error: {0}")]
    StateTransition(#[from] StateTransitionError),
    /// An error with AWS SQS
    #[error("SQS error: {0}")]
    Sqs(String),
    /// An error with the darkpool client
    #[error("darkpool client error: {0}")]
    DarkpoolClient(#[from] DarkpoolClientError),
    /// An error de/serializing a value
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl<E, R> From<SdkError<E, R>> for HandlerError {
    fn from(value: SdkError<E, R>) -> Self {
        HandlerError::Sqs(value.to_string())
    }
}
