//! Error types for the execution client

use price_reporter_client::error::PriceReporterClientError;
use warp::reject::Reject;

/// An error returned by the execution client
#[derive(Debug, Clone, thiserror::Error)]
pub enum ExecutionClientError {
    /// A custom error
    #[error("custom error: {0}")]
    Custom(String),
    /// An error returned by the execution client
    #[error("http error: {0}")]
    Http(String),
    /// An error interacting with the chain
    #[error("on-chain error: {0}")]
    OnChain(String),
    /// An error parsing a value
    #[error("parse error: {0}")]
    Parse(String),
    /// An error returned by the price reporter
    #[error("price reporter error: {0}")]
    PriceReporter(#[from] PriceReporterClientError),
    /// An error validating a quote
    #[error("quote validation error: {0}")]
    QuoteValidation(String),
    /// An error converting a venue quote to an executable quote
    #[error("quote conversion error: {0}")]
    QuoteConversion(String),
}

impl ExecutionClientError {
    /// Create a new custom error
    #[allow(clippy::needless_pass_by_value)]
    pub fn custom<T: ToString>(e: T) -> Self {
        ExecutionClientError::Custom(e.to_string())
    }

    /// Create a new http error
    #[allow(clippy::needless_pass_by_value)]
    pub fn http<T: ToString>(e: T) -> Self {
        ExecutionClientError::Http(e.to_string())
    }

    /// Create a new onchain error
    #[allow(clippy::needless_pass_by_value)]
    pub fn onchain<T: ToString>(e: T) -> Self {
        ExecutionClientError::OnChain(e.to_string())
    }

    /// Create a new parse error
    #[allow(clippy::needless_pass_by_value)]
    pub fn parse<T: ToString>(e: T) -> Self {
        ExecutionClientError::Parse(e.to_string())
    }

    /// Create a new quote validation error
    #[allow(clippy::needless_pass_by_value)]
    pub fn quote_validation<T: ToString>(e: T) -> Self {
        ExecutionClientError::QuoteValidation(e.to_string())
    }

    /// Create a new quote conversion error
    #[allow(clippy::needless_pass_by_value)]
    pub fn quote_conversion<T: ToString>(e: T) -> Self {
        ExecutionClientError::QuoteConversion(e.to_string())
    }
}

impl Reject for ExecutionClientError {}

impl From<reqwest::Error> for ExecutionClientError {
    fn from(e: reqwest::Error) -> Self {
        ExecutionClientError::http(e)
    }
}
