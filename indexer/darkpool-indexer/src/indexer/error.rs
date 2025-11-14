//! Top-level indexer error definitions

use aws_sdk_sqs::error::SdkError;

use crate::{
    chain_event_listener::error::ChainEventListenerError,
    darkpool_client::error::DarkpoolClientError, db::error::DbError,
    state_transitions::error::StateTransitionError,
};

/// Indexer errors
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// An error with AWS SQS
    #[error("SQS error: {0}")]
    Sqs(String),
    /// An error in the RPC client
    #[error("RPC client error: {0}")]
    Rpc(String),
    /// An error in the darkpool client
    #[error("darkpool client error: {0}")]
    DarkpoolClient(#[from] DarkpoolClientError),
    /// An invalid selector was encountered
    #[error("invalid selector: {0}")]
    InvalidSelector(String),
    /// An invalid settlement bundle was encountered
    #[error("invalid settlement bundle: {0}")]
    InvalidSettlementBundle(String),
    /// An error in the database client
    #[error("database error: {0}")]
    Db(#[from] DbError),
    /// An error in the state transition applicator
    #[error("state transition error: {0}")]
    StateTransition(#[from] StateTransitionError),
    /// An error in the chain event listener
    #[error("chain event listener error: {0}")]
    ChainEventListener(#[from] ChainEventListenerError),
    /// An error parsing a value
    #[error("parse error: {0}")]
    Parse(String),
    /// An error de/serializing a value
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[allow(clippy::needless_pass_by_value)]
impl IndexerError {
    /// Create a new RPC error
    pub fn rpc<T: ToString>(msg: T) -> Self {
        Self::Rpc(msg.to_string())
    }

    /// Create a new parse error
    pub fn parse<T: ToString>(msg: T) -> Self {
        Self::Parse(msg.to_string())
    }

    /// Create a new invalid settlement bundle error
    pub fn invalid_settlement_bundle<T: ToString>(msg: T) -> Self {
        Self::InvalidSettlementBundle(msg.to_string())
    }
}

impl<E, R> From<SdkError<E, R>> for IndexerError {
    fn from(value: SdkError<E, R>) -> Self {
        IndexerError::Sqs(value.to_string())
    }
}
