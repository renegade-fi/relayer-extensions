//! Error types for the price reporter client

use thiserror::Error;

use crate::http_utils::HttpError;

/// Error type for price reporter operations
#[derive(Debug, Error)]
pub enum PriceReporterError {
    /// Setup error
    #[error("Setup error: {0}")]
    Setup(String),

    /// Parsing error
    #[error("Parsing error: {0}")]
    Parsing(String),

    /// HTTP error
    #[error("HTTP error: {0}")]
    Http(HttpError),

    /// WebSocket error
    #[error("WebSocket error: {0}")]
    WebSocket(String),
}

impl From<HttpError> for PriceReporterError {
    fn from(err: HttpError) -> Self {
        Self::Http(err)
    }
}

impl PriceReporterError {
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

    /// Create a new web socket error
    #[allow(clippy::needless_pass_by_value)]
    pub fn websocket<T: ToString>(msg: T) -> Self {
        Self::WebSocket(msg.to_string())
    }
}
