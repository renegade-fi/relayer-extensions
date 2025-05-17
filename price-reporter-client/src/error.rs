//! Error types for the price reporter client

use thiserror::Error;

/// Error type for price reporter operations
#[derive(Debug, Error, Clone)]
pub enum PriceReporterClientError {
    /// Setup error
    #[error("Setup error: {0}")]
    Setup(String),

    /// Parsing error
    #[error("Parsing error: {0}")]
    Parsing(String),

    /// Conversion error
    #[error("Conversion error: {0}")]
    Conversion(String),

    /// HTTP error
    #[error("HTTP error: {0}")]
    Http(String),

    /// WebSocket error
    #[error("WebSocket error: {0}")]
    WebSocket(String),

    /// Custom error
    #[error("Custom error: {0}")]
    Custom(String),
}

impl PriceReporterClientError {
    /// Create a new setup error
    #[allow(clippy::needless_pass_by_value)]
    pub fn setup<T: ToString>(msg: T) -> Self {
        Self::Setup(msg.to_string())
    }

    /// Create a new parsing error
    #[allow(clippy::needless_pass_by_value)]
    pub fn parsing<T: ToString>(msg: T) -> Self {
        Self::Parsing(msg.to_string())
    }

    /// Create a new conversion error
    #[allow(clippy::needless_pass_by_value)]
    pub fn conversion<T: ToString>(msg: T) -> Self {
        Self::Conversion(msg.to_string())
    }

    /// Create a new HTTP error
    #[allow(clippy::needless_pass_by_value)]
    pub fn http<T: ToString>(msg: T) -> Self {
        Self::Http(msg.to_string())
    }

    /// Create a new web socket error
    #[allow(clippy::needless_pass_by_value)]
    pub fn websocket<T: ToString>(msg: T) -> Self {
        Self::WebSocket(msg.to_string())
    }

    /// Create a new custom error
    #[allow(clippy::needless_pass_by_value)]
    pub fn custom<T: ToString>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}
