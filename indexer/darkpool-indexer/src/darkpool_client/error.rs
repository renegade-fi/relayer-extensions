//! Error definitions for the darkpool client

/// Darkpool client errors
#[derive(Debug, thiserror::Error)]
pub enum DarkpoolClientError {
    /// An error fetching a call trace
    #[error("call trace error: {0}")]
    CallTrace(String),
    /// A spent nullifier was not found in a call trace
    #[error("spent nullifier not found")]
    NullifierNotFound,
    /// An invalid selector was provided
    #[error("invalid selector: {0}")]
    InvalidSelector(String),
    /// An error decoding calldata
    #[error("calldata decoding error: {0}")]
    CalldataDecode(String),
}

#[allow(clippy::needless_pass_by_value)]
impl DarkpoolClientError {
    /// Create a new call trace error
    pub fn call_trace<T: ToString>(msg: T) -> Self {
        Self::CallTrace(msg.to_string())
    }

    /// Create a new calldata decoding error
    pub fn calldata_decode<T: ToString>(msg: T) -> Self {
        Self::CalldataDecode(msg.to_string())
    }
}
