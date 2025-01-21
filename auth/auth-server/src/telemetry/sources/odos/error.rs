//! Error types for the Odos client

use thiserror::Error;

/// An error with the Odos client
#[derive(Debug, Error)]
pub enum OdosError {
    /// An error with the input
    #[error("Invalid input: {0}")]
    Input(String),
}
