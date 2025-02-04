//! Error types for the Odos client

use thiserror::Error;

/// An error with the Odos client
#[derive(Debug, Error)]
pub enum OdosError {
    /// An error with the input
    #[error("Invalid input: {0}")]
    Input(String),
    /// An error with the Odos API
    #[error("Odos API error: {0}")]
    Api(String),
}

impl OdosError {
    /// Create a new API error
    #[allow(clippy::needless_pass_by_value)]
    pub fn api<T: ToString>(msg: T) -> Self {
        Self::Api(msg.to_string())
    }
}
