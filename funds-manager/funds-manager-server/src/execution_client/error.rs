//! Error types for the execution client

use std::{error::Error, fmt::Display};

use price_reporter_client::error::PriceReporterClientError;
use warp::reject::Reject;

/// An error returned by the execution client
#[derive(Debug, Clone)]
pub enum ExecutionClientError {
    /// An error interacting with the chain
    OnChain(String),
    /// An error returned by the execution client
    Http(String),
    /// An error parsing a value
    Parse(String),
    /// An error returned by the price reporter
    PriceReporter(PriceReporterClientError),
    /// A custom error
    Custom(String),
}

impl ExecutionClientError {
    /// Create a new onchain error
    #[allow(clippy::needless_pass_by_value)]
    pub fn onchain<T: ToString>(e: T) -> Self {
        ExecutionClientError::OnChain(e.to_string())
    }

    /// Create a new http error
    #[allow(clippy::needless_pass_by_value)]
    pub fn http<T: ToString>(e: T) -> Self {
        ExecutionClientError::Http(e.to_string())
    }

    /// Create a new parse error
    #[allow(clippy::needless_pass_by_value)]
    pub fn parse<T: ToString>(e: T) -> Self {
        ExecutionClientError::Parse(e.to_string())
    }

    /// Create a new custom error
    #[allow(clippy::needless_pass_by_value)]
    pub fn custom<T: ToString>(e: T) -> Self {
        ExecutionClientError::Custom(e.to_string())
    }
}

impl Display for ExecutionClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            ExecutionClientError::OnChain(e) => format!("Onchain error: {e}"),
            ExecutionClientError::Http(e) => format!("HTTP error: {e}"),
            ExecutionClientError::Parse(e) => format!("Parse error: {e}"),
            ExecutionClientError::PriceReporter(e) => format!("Price reporter error: {e}"),
            ExecutionClientError::Custom(e) => format!("Custom error: {e}"),
        };

        write!(f, "{}", msg)
    }
}
impl Error for ExecutionClientError {}
impl Reject for ExecutionClientError {}

impl From<reqwest::Error> for ExecutionClientError {
    fn from(e: reqwest::Error) -> Self {
        ExecutionClientError::http(e)
    }
}

impl From<PriceReporterClientError> for ExecutionClientError {
    fn from(e: PriceReporterClientError) -> Self {
        ExecutionClientError::PriceReporter(e)
    }
}
