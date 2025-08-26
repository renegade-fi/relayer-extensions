//! Defines the error types for the transaction driver.
use thiserror::Error;

/// The generic tx driver error
#[derive(Debug, Error)]
pub enum DriverError {
    /// The chain client error.
    #[error("chain client error: {0}")]
    Chain(String),
}

impl From<eyre::Report> for DriverError {
    fn from(err: eyre::Report) -> Self {
        DriverError::Chain(err.to_string())
    }
}
