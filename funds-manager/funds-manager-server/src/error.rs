//! Error types for the funds manager

use std::{error::Error, fmt::Display};

use price_reporter_client::error::PriceReporterClientError;
use warp::reject::Reject;

use fireblocks_sdk::{apis::Error as FireblocksApiError, FireblocksError};

/// The error type emitted by the funds manager
#[derive(Debug, Clone)]
pub enum FundsManagerError {
    /// An error with the arbitrum client
    Arbitrum(String),
    /// An error with a database query
    Db(String),
    /// An error with Fireblocks operations
    Fireblocks(String),
    /// An error executing an HTTP request
    Http(String),
    /// An error parsing a value
    Parse(String),
    /// An error with AWS secrets manager
    SecretsManager(String),
    /// An error with AWS S3
    S3(String),
    /// An error with a JSON-RPC request
    JsonRpc(String),
    /// An error with the price reporter
    PriceReporter(PriceReporterClientError),
    /// A miscellaneous error
    Custom(String),
}

#[allow(clippy::needless_pass_by_value)]
impl FundsManagerError {
    /// Create an arbitrum error
    pub fn arbitrum<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::Arbitrum(msg.to_string())
    }

    /// Create a database error
    pub fn db<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::Db(msg.to_string())
    }

    /// Create a Fireblocks error
    pub fn fireblocks<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::Fireblocks(msg.to_string())
    }

    /// Create an HTTP error
    pub fn http<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::Http(msg.to_string())
    }

    /// Create a parse error
    pub fn parse<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::Parse(msg.to_string())
    }

    /// Create a secrets manager error
    pub fn secrets_manager<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::SecretsManager(msg.to_string())
    }

    /// Create a S3 error
    pub fn s3<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::S3(msg.to_string())
    }

    /// Create a JSON-RPC error
    pub fn json_rpc<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::JsonRpc(msg.to_string())
    }

    /// Create a custom error
    pub fn custom<T: ToString>(msg: T) -> FundsManagerError {
        FundsManagerError::Custom(msg.to_string())
    }
}

impl Display for FundsManagerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FundsManagerError::Arbitrum(e) => write!(f, "Arbitrum error: {}", e),
            FundsManagerError::Db(e) => write!(f, "Database error: {}", e),
            FundsManagerError::Http(e) => write!(f, "HTTP error: {}", e),
            FundsManagerError::Parse(e) => write!(f, "Parse error: {}", e),
            FundsManagerError::SecretsManager(e) => write!(f, "Secrets manager error: {}", e),
            FundsManagerError::S3(e) => write!(f, "S3 error: {}", e),
            FundsManagerError::JsonRpc(e) => write!(f, "JSON-RPC error: {}", e),
            FundsManagerError::Custom(e) => write!(f, "Uncategorized error: {}", e),
            FundsManagerError::Fireblocks(e) => write!(f, "Fireblocks error: {}", e),
            FundsManagerError::PriceReporter(e) => write!(f, "Price reporter error: {}", e),
        }
    }
}
impl Error for FundsManagerError {}
impl Reject for FundsManagerError {}

impl<T> From<FireblocksApiError<T>> for FundsManagerError {
    fn from(error: FireblocksApiError<T>) -> Self {
        FundsManagerError::Fireblocks(error.to_string())
    }
}

impl From<FireblocksError> for FundsManagerError {
    fn from(error: FireblocksError) -> Self {
        FundsManagerError::Fireblocks(error.to_string())
    }
}

impl From<PriceReporterClientError> for FundsManagerError {
    fn from(error: PriceReporterClientError) -> Self {
        FundsManagerError::PriceReporter(error)
    }
}

/// API-specific error type
#[derive(Debug)]
pub enum ApiError {
    /// Error during fee indexing
    IndexingError(String),
    /// Error during fee redemption
    RedemptionError(String),
    /// Internal server error
    InternalError(String),
    /// Bad request error
    BadRequest(String),
    /// Unauthenticated error
    Unauthenticated(String),
}

impl Reject for ApiError {}

impl Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::IndexingError(e) => write!(f, "Indexing error: {}", e),
            ApiError::RedemptionError(e) => write!(f, "Redemption error: {}", e),
            ApiError::InternalError(e) => write!(f, "Internal error: {}", e),
            ApiError::BadRequest(e) => write!(f, "Bad request: {}", e),
            ApiError::Unauthenticated(e) => write!(f, "Unauthenticated: {}", e),
        }
    }
}

impl Error for ApiError {}
