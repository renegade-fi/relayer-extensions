//! Error types for the auth server

use thiserror::Error;

/// Custom error type for server errors
#[derive(Error, Debug)]
pub enum AuthServerError {
    /// Database connection error
    #[error("Database connection error: {0}")]
    DatabaseConnection(String),

    /// Encryption error
    #[error("Encryption error: {0}")]
    Encryption(String),

    /// Decryption error
    #[error("Decryption error: {0}")]
    Decryption(String),
}

impl AuthServerError {
    /// Create a new database connection error
    pub fn db<T: ToString>(msg: T) -> Self {
        Self::DatabaseConnection(msg.to_string())
    }

    /// Create a new encryption error
    pub fn encryption<T: ToString>(msg: T) -> Self {
        Self::Encryption(msg.to_string())
    }

    /// Create a new decryption error
    pub fn decryption<T: ToString>(msg: T) -> Self {
        Self::Decryption(msg.to_string())
    }
}

impl warp::reject::Reject for AuthServerError {}
