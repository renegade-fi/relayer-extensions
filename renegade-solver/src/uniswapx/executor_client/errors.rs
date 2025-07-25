//! Possible errors thrown by the executor client

use alloy::providers::PendingTransactionError;
use alloy_contract::Error as SolError;

#[derive(Debug, thiserror::Error)]
/// The error type returned by the executor client configuration interface
pub enum ExecutorConfigError {
    #[error("Failed to parse RPC URL: {0}")]
    /// Error thrown when the RPC client fails to initialize
    RpcClientInitialization(String),

    #[error("Failed to parse contract address: {0}")]
    /// Error thrown when a contract address can't be parsed
    AddressParsing(String),
}

#[derive(Debug, thiserror::Error)]
/// The error type returned by the executor client
pub enum ExecutorError {
    #[error("Configuration error: {0}")]
    /// Error thrown when the executor client configuration fails
    Config(#[from] ExecutorConfigError),

    #[error("Contract interaction error: {0}")]
    /// Error thrown when a contract call fails
    ContractInteraction(String),

    #[error("RPC error: {0}")]
    /// An error interacting with the lower level rpc client
    Rpc(String),

    #[error("Pending transaction error: {0}")]
    /// Error thrown when a transaction fails
    PendingTransaction(String),
}

impl ExecutorError {
    /// Create a new contract interaction error
    #[allow(clippy::needless_pass_by_value)]
    pub fn contract_interaction<T: ToString>(msg: T) -> Self {
        Self::ContractInteraction(msg.to_string())
    }
    /// Create a new RPC error
    #[allow(clippy::needless_pass_by_value)]
    pub fn rpc<T: ToString>(msg: T) -> Self {
        Self::Rpc(msg.to_string())
    }
}

impl From<SolError> for ExecutorError {
    fn from(e: SolError) -> Self {
        Self::ContractInteraction(e.to_string())
    }
}

impl From<PendingTransactionError> for ExecutorError {
    fn from(e: PendingTransactionError) -> Self {
        Self::PendingTransaction(e.to_string())
    }
}
