//! Error types for the execution client

use std::fmt::Display;

/// An error returned by the execution client
#[derive(Debug, Clone)]
pub enum ExecutionClientError {
    /// An error returned by the execution client
    Http(String),
    /// An error parsing a value
    Parse(String),
}

impl ExecutionClientError {
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
}

impl Display for ExecutionClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let msg = match self {
            ExecutionClientError::Http(e) => format!("HTTP error: {e}"),
            ExecutionClientError::Parse(e) => format!("Parse error: {e}"),
        };

        write!(f, "{}", msg)
    }
}

impl From<reqwest::Error> for ExecutionClientError {
    fn from(e: reqwest::Error) -> Self {
        ExecutionClientError::http(e)
    }
}