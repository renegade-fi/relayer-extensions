//! Error types for the auth server

use thiserror::Error;

use crate::server::price_reporter_client::error::PriceReporterError;
use crate::ApiError;

/// Custom error type for server errors
#[derive(Error, Debug)]
pub enum AuthServerError {
    /// API key inactive
    #[error("API key inactive")]
    ApiKeyInactive,
    /// Database connection error
    #[error("Database connection error: {0}")]
    DatabaseConnection(String),
    /// Redis connection error
    #[error("Redis connection error: {0}")]
    RedisConnection(String),
    /// Encryption error
    #[error("Encryption error: {0}")]
    Encryption(String),
    /// Decryption error
    #[error("Decryption error: {0}")]
    Decryption(String),
    /// Error serializing or deserializing a stored value
    #[error("Error serializing/deserializing a stored value: {0}")]
    Serde(String),
    /// Error setting up the auth server
    #[error("Error setting up the auth server: {0}")]
    Setup(String),
    /// Unauthorized
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    /// Gas sponsorship error
    #[error("Gas sponsorship error: {0}")]
    GasSponsorship(String),
    /// An error signing a message
    #[error("Signing error: {0}")]
    Signing(String),
    /// An error comparing quotes
    #[error("Quote comparison error: {0}")]
    QuoteComparison(String),
    /// A miscellaneous error
    #[error("Error: {0}")]
    Custom(String),
    /// Price reporter error
    #[error("Price reporter error: {0}")]
    PriceReporter(#[from] PriceReporterError),
    /// Arbitrum client error
    #[error("Arbitrum client error: {0}")]
    ArbitrumClient(String),
    /// Gas cost sampler error
    #[error("Gas cost sampler error: {0}")]
    GasCostSampler(String),
}

impl AuthServerError {
    /// Create a new arbitrum client error
    #[allow(clippy::needless_pass_by_value)]
    pub fn arbitrum<T: ToString>(msg: T) -> Self {
        Self::ArbitrumClient(msg.to_string())
    }

    /// Create a new database connection error
    #[allow(clippy::needless_pass_by_value)]
    pub fn db<T: ToString>(msg: T) -> Self {
        Self::DatabaseConnection(msg.to_string())
    }

    /// Create a new redis connection error
    #[allow(clippy::needless_pass_by_value)]
    pub fn redis<T: ToString>(msg: T) -> Self {
        Self::RedisConnection(msg.to_string())
    }

    /// Create a new encryption error
    #[allow(clippy::needless_pass_by_value)]
    pub fn encryption<T: ToString>(msg: T) -> Self {
        Self::Encryption(msg.to_string())
    }

    /// Create a new decryption error
    #[allow(clippy::needless_pass_by_value)]
    pub fn decryption<T: ToString>(msg: T) -> Self {
        Self::Decryption(msg.to_string())
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

    /// Create a new unauthorized error
    #[allow(clippy::needless_pass_by_value)]
    pub fn unauthorized<T: ToString>(msg: T) -> Self {
        Self::Unauthorized(msg.to_string())
    }

    /// Create a new gas sponsorship error
    #[allow(clippy::needless_pass_by_value)]
    pub fn gas_sponsorship<T: ToString>(msg: T) -> Self {
        Self::GasSponsorship(msg.to_string())
    }

    /// Create a new signing error
    #[allow(clippy::needless_pass_by_value)]
    pub fn signing<T: ToString>(msg: T) -> Self {
        Self::Signing(msg.to_string())
    }

    /// Create a new quote comparison error
    #[allow(clippy::needless_pass_by_value)]
    pub fn quote_comparison<T: ToString>(msg: T) -> Self {
        Self::QuoteComparison(msg.to_string())
    }

    /// Create a new custom error
    #[allow(clippy::needless_pass_by_value)]
    pub fn custom<T: ToString>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }

    /// Create a new arbitrum client error
    #[allow(clippy::needless_pass_by_value)]
    pub fn arbitrum_client<T: ToString>(msg: T) -> Self {
        Self::ArbitrumClient(msg.to_string())
    }

    /// Create a new gas cost sampler error
    #[allow(clippy::needless_pass_by_value)]
    pub fn gas_cost_sampler<T: ToString>(msg: T) -> Self {
        Self::GasCostSampler(msg.to_string())
    }
}

impl warp::reject::Reject for AuthServerError {}

impl From<AuthServerError> for ApiError {
    fn from(err: AuthServerError) -> Self {
        match err {
            AuthServerError::ApiKeyInactive | AuthServerError::Unauthorized(_) => {
                ApiError::Unauthorized
            },
            _ => ApiError::InternalError(err.to_string()),
        }
    }
}
