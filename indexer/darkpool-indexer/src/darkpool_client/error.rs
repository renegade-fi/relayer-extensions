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
    /// A nullifier was not found in a call trace
    #[error("nullifier not found")]
    NullifierNotFound,
    /// A public intent hash was not found in a call trace
    #[error("public intent hash not found")]
    PublicIntentHashNotFound,
    /// An error ABI-decoding a value
    #[error("ABI decoding error: {0}")]
    AbiDecoding(String),
}

#[allow(clippy::needless_pass_by_value)]
impl DarkpoolClientError {
    /// Create a new RPC error
    pub fn rpc<T: ToString>(msg: T) -> Self {
        Self::Rpc(msg.to_string())
    }

    /// Create a new ABI decoding error
    pub fn abi_decoding<T: ToString>(msg: T) -> Self {
        Self::AbiDecoding(msg.to_string())
    }
}
