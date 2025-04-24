//! Defines error types for the on-chain event listener

use std::{error::Error, fmt::Display};

use renegade_arbitrum_client::errors::ArbitrumClientError;

use crate::error::AuthServerError;

/// The error type that the event listener emits
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum OnChainEventListenerError {
    /// An error executing some method in the Arbitrum client
    Arbitrum(String),
    /// An RPC error with the provider
    Rpc(String),
    /// Error setting up the on-chain event listener
    Setup(String),
    /// The stream unexpectedly stopped
    StreamEnded,
    /// Auth server error
    AuthServer(String),
}

impl OnChainEventListenerError {
    /// Create a new arbitrum error
    #[allow(clippy::needless_pass_by_value)]
    pub fn arbitrum<T: ToString>(e: T) -> Self {
        OnChainEventListenerError::Arbitrum(e.to_string())
    }

    /// Create a new auth server error
    #[allow(clippy::needless_pass_by_value)]
    pub fn auth_server<T: ToString>(e: T) -> Self {
        OnChainEventListenerError::AuthServer(e.to_string())
    }
}

impl Display for OnChainEventListenerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl Error for OnChainEventListenerError {}

impl From<ArbitrumClientError> for OnChainEventListenerError {
    fn from(e: ArbitrumClientError) -> Self {
        OnChainEventListenerError::arbitrum(e)
    }
}

impl<E: Display> From<alloy::transports::RpcError<E>> for OnChainEventListenerError {
    fn from(e: alloy::transports::RpcError<E>) -> Self {
        OnChainEventListenerError::Rpc(e.to_string())
    }
}

impl From<alloy::sol_types::Error> for OnChainEventListenerError {
    fn from(e: alloy::sol_types::Error) -> Self {
        OnChainEventListenerError::Rpc(e.to_string())
    }
}

impl From<AuthServerError> for OnChainEventListenerError {
    fn from(err: AuthServerError) -> Self {
        OnChainEventListenerError::auth_server(err)
    }
}
