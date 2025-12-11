//! Top-level indexer error definitions

use std::fmt::Display;

use alloy::hex;
use uuid::Uuid;

use crate::{
    chain_event_listener::error::ChainEventListenerError,
    darkpool_client::error::DarkpoolClientError, db::error::DbError,
    message_queue::error::MessageQueueError, state_transitions::error::StateTransitionError,
};

/// Indexer errors
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    /// An error setting up telemetry
    #[error("error setting up telemetry: {0}")]
    Telemetry(String),
    /// An error with the message queue
    #[error("message queue error: {0}")]
    MessageQueue(#[from] MessageQueueError),
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
    /// An invalid obligation bundle was encountered
    #[error("invalid obligation bundle: {0}")]
    InvalidObligationBundle(String),
    /// An invalid output balance bundle was encountered
    #[error("invalid output balance bundle: {0}")]
    InvalidOutputBalanceBundle(String),
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
    /// An error backfilling a user's state
    #[error("error backfilling state for account {0}")]
    Backfill(Uuid),
}

#[allow(clippy::needless_pass_by_value)]
impl IndexerError {
    /// Create a new telemetry error
    pub fn telemetry<T: ToString>(msg: T) -> Self {
        Self::Telemetry(msg.to_string())
    }

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

    /// Create a new invalid obligation bundle error
    pub fn invalid_obligation_bundle<T: ToString>(msg: T) -> Self {
        Self::InvalidObligationBundle(msg.to_string())
    }

    /// Create a new invalid output balance bundle error
    pub fn invalid_output_balance_bundle<T: ToString>(msg: T) -> Self {
        Self::InvalidOutputBalanceBundle(msg.to_string())
    }

    /// Create a new invalid selector error
    pub fn invalid_selector<T: AsRef<[u8]>>(selector: T) -> Self {
        Self::InvalidSelector(hex::encode_prefixed(selector))
    }
}

impl<E: Display> From<alloy::transports::RpcError<E>> for IndexerError {
    fn from(e: alloy::transports::RpcError<E>) -> Self {
        IndexerError::Rpc(e.to_string())
    }
}
