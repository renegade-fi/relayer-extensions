//! Error definitions for the darkpool client

/// Darkpool client errors
#[derive(Debug, thiserror::Error)]
pub enum DarkpoolClientError {
    /// An error interacting with the RPC client
    #[error("RPC client error: {0}")]
    Rpc(String),
    /// A recovery ID was not found in a call trace
    #[error("recovery ID not found")]
    RecoveryIdNotFound,
    /// An invalid selector was provided
    #[error("invalid selector: {0}")]
    InvalidSelector(String),
    /// An error decoding calldata
    #[error("calldata decoding error: {0}")]
    CalldataDecode(String),
}

#[allow(clippy::needless_pass_by_value)]
impl DarkpoolClientError {
    /// Create a new RPC error
    pub fn rpc<T: ToString>(msg: T) -> Self {
        Self::Rpc(msg.to_string())
    }

    /// Create a new calldata decoding error
    pub fn calldata_decode<T: ToString>(msg: T) -> Self {
        Self::CalldataDecode(msg.to_string())
    }
}
