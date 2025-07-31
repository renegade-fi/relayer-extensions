//! Possible errors thrown by the executor client

use alloy::providers::PendingTransactionError;
use alloy_contract::Error as ContractError;
use alloy_primitives::U256;
use alloy_sol_types::Error as SolTypeError;

#[derive(Debug, thiserror::Error)]
/// The error type returned by the executor client configuration interface
pub enum ExecutorConfigError {
    #[error("Failed to parse contract address: {0}")]
    /// Error thrown when a contract address can't be parsed
    AddressParsing(String),
    #[error("Failed to parse RPC URL: {0}")]
    /// Error thrown when the RPC client fails to initialize
    RpcClientInitialization(String),
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
    /// Conversion error from U256 to u128
    #[error("Invalid u256 to u128 conversion: {0}")]
    InvalidU256(U256),
    #[error("Invalid calldata: {0}")]
    /// Error thrown when the calldata is invalid
    InvalidCalldata(SolTypeError),
    #[error("Pending transaction error: {0}")]
    /// Error thrown when a transaction fails
    PendingTransaction(String),
    #[error("RPC error: {0}")]
    /// An error interacting with the lower level rpc client
    Rpc(String),
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

impl From<ContractError> for ExecutorError {
    fn from(e: ContractError) -> Self {
        Self::ContractInteraction(e.to_string())
    }
}

impl From<PendingTransactionError> for ExecutorError {
    fn from(e: PendingTransactionError) -> Self {
        Self::PendingTransaction(e.to_string())
    }
}

impl From<SolTypeError> for ExecutorError {
    fn from(e: SolTypeError) -> Self {
        Self::InvalidCalldata(e)
    }
}

impl From<U256> for ExecutorError {
    fn from(u: U256) -> Self {
        ExecutorError::InvalidU256(u)
    }
}
