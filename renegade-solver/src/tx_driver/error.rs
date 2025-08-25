//! Defines the error types for the transaction driver.
use thiserror::Error;

/// The generic tx driver error
#[derive(Debug, Error)]
pub enum DriverError {
    /// The chain client error.
    #[error("chain client error: {0}")]
    Chain(#[from] eyre::Report),
}
