//! Error types for the auth server

use thiserror::Error;

use crate::ApiError;
use price_reporter_client::error::PriceReporterClientError;

/// The message indicating no match was found
const ERR_NO_MATCH_FOUND: &str = "No match found";

/// Custom error type for server errors
#[derive(Error, Debug, Clone)]
pub enum AuthServerError {
    /// API key inactive
    #[error("API key inactive")]
    ApiKeyInactive,
    /// A bad request error
    #[error("Bad request: {0}")]
    BadRequest(String),
    /// Bundle store error
    #[error("Bundle store error: {0}")]
    BundleStore(String),
    /// A miscellaneous error
    #[error("Error: {0}")]
    Custom(String),
    /// Darkpool client error
    #[error("Darkpool client error: {0}")]
    DarkpoolClient(String),
    /// Database connection error
    #[error("Database connection error: {0}")]
    DatabaseConnection(String),
    /// Decryption error
    #[error("Decryption error: {0}")]
    Decryption(String),
    /// Encryption error
    #[error("Encryption error: {0}")]
    Encryption(String),
    /// Gas cost sampler error
    #[error("Gas cost sampler error: {0}")]
    GasCostSampler(String),
    /// Gas sponsorship error
    #[error("Gas sponsorship error: {0}")]
    GasSponsorship(String),
    /// A no content (HTTP 204) error
    #[error("No content: {0}")]
    NoContent(String),
    /// Price reporter error
    #[error("Price reporter error: {0}")]
    PriceReporter(#[from] PriceReporterClientError),
    /// A rate limit error
    #[error("Rate limited")]
    RateLimit,
    /// Redis connection error
    #[error("Redis connection error: {0}")]
    RedisConnection(String),
    /// Error serializing or deserializing a stored value
    #[error("Error serializing/deserializing a stored value: {0}")]
    Serde(String),
    /// Error setting up the auth server
    #[error("Error setting up the auth server: {0}")]
    Setup(String),
    /// An error signing a message
    #[error("Signing error: {0}")]
    Signing(String),
    /// Unauthorized
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
}

impl AuthServerError {
    /// Create a new bad request error
    #[allow(clippy::needless_pass_by_value)]
    pub fn bad_request<T: ToString>(msg: T) -> Self {
        Self::BadRequest(msg.to_string())
    }

    /// Create a new custom error
    #[allow(clippy::needless_pass_by_value)]
    pub fn custom<T: ToString>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }

    /// Create a new darkpool client error
    #[allow(clippy::needless_pass_by_value)]
    pub fn darkpool_client<T: ToString>(msg: T) -> Self {
        Self::DarkpoolClient(msg.to_string())
    }

    /// Create a new database connection error
    #[allow(clippy::needless_pass_by_value)]
    pub fn db<T: ToString>(msg: T) -> Self {
        Self::DatabaseConnection(msg.to_string())
    }

    /// Create a new decryption error
    #[allow(clippy::needless_pass_by_value)]
    pub fn decryption<T: ToString>(msg: T) -> Self {
        Self::Decryption(msg.to_string())
    }

    /// Create a new encryption error
    #[allow(clippy::needless_pass_by_value)]
    pub fn encryption<T: ToString>(msg: T) -> Self {
        Self::Encryption(msg.to_string())
    }

    /// Create a new gas cost sampler error
    #[allow(clippy::needless_pass_by_value)]
    pub fn gas_cost_sampler<T: ToString>(msg: T) -> Self {
        Self::GasCostSampler(msg.to_string())
    }

    /// Create a new gas sponsorship error
    #[allow(clippy::needless_pass_by_value)]
    pub fn gas_sponsorship<T: ToString>(msg: T) -> Self {
        Self::GasSponsorship(msg.to_string())
    }

    /// Create a new no content error
    #[allow(clippy::needless_pass_by_value)]
    pub fn no_content<T: ToString>(msg: T) -> Self {
        Self::NoContent(msg.to_string())
    }

    /// Create a new no match found error
    pub fn no_match_found() -> Self {
        Self::no_content(ERR_NO_MATCH_FOUND)
    }

    /// Create a new redis connection error
    #[allow(clippy::needless_pass_by_value)]
    pub fn redis<T: ToString>(msg: T) -> Self {
        Self::RedisConnection(msg.to_string())
    }

    /// Create a new serde error
    #[allow(clippy::needless_pass_by_value)]
    pub fn serde<T: ToString>(msg: T) -> Self {
        Self::Serde(msg.to_string())
    }

    /// Create a new setup error
    #[allow(clippy::needless_pass_by_value)]
    pub fn setup<T: ToString>(msg: T) -> Self {
        Self::Setup(msg.to_string())
    }

    /// Create a new signing error
    #[allow(clippy::needless_pass_by_value)]
    pub fn signing<T: ToString>(msg: T) -> Self {
        Self::Signing(msg.to_string())
    }

    /// Create a new unauthorized error
    #[allow(clippy::needless_pass_by_value)]
    pub fn unauthorized<T: ToString>(msg: T) -> Self {
        Self::Unauthorized(msg.to_string())
    }
}

impl warp::reject::Reject for AuthServerError {}

impl From<AuthServerError> for ApiError {
    fn from(err: AuthServerError) -> Self {
        match err {
            AuthServerError::ApiKeyInactive | AuthServerError::Unauthorized(_) => {
                ApiError::Unauthorized
            },
            AuthServerError::BadRequest(e) | AuthServerError::Serde(e) => ApiError::BadRequest(e),
            AuthServerError::RateLimit => ApiError::TooManyRequests,
            AuthServerError::NoContent(e) => ApiError::NoContent(e),
            _ => ApiError::InternalError(err.to_string()),
        }
    }
}

impl From<redis::RedisError> for AuthServerError {
    fn from(err: redis::RedisError) -> Self {
        Self::redis(err)
    }
}

impl From<alloy_sol_types::Error> for AuthServerError {
    fn from(err: alloy_sol_types::Error) -> Self {
        Self::custom(err)
    }
}
