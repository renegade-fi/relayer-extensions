//! Defines error types for the chain event listener

use std::fmt::Display;

use crate::message_queue::error::MessageQueueError;

/// The error type that the chain event listener emits
#[derive(Debug, thiserror::Error)]
pub enum ChainEventListenerError {
    /// An error with the RPC client
    #[error("RPC client error: {0}")]
    Rpc(String),
    /// An error with the message queue
    #[error("message queue error: {0}")]
    MessageQueue(#[from] MessageQueueError),
    /// An error de/serializing a value
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[allow(clippy::needless_pass_by_value)]
impl ChainEventListenerError {
    /// Create a new RPC error
    pub fn rpc<T: ToString>(msg: T) -> Self {
        Self::Rpc(msg.to_string())
    }
}

impl<E: Display> From<alloy::transports::RpcError<E>> for ChainEventListenerError {
    fn from(e: alloy::transports::RpcError<E>) -> Self {
        ChainEventListenerError::Rpc(e.to_string())
    }
}
