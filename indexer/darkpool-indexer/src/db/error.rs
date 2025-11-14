//! Database error definitions

/// Database errors
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// An error setting up the database client
    #[error("client setup error: {0}")]
    ClientSetup(String),
    /// An error obtaining a connection from the pool
    #[error("pool connection error: {0}")]
    PoolConnection(String),
    /// An arbitrary Diesel error
    #[error("Diesel error: {0}")]
    DieselError(#[from] diesel::result::Error),
}

#[allow(clippy::needless_pass_by_value)]
impl DbError {
    /// Create a new database client setup error
    pub fn client_setup<T: ToString>(msg: T) -> Self {
        Self::ClientSetup(msg.to_string())
    }

    /// Create a new pool connection error
    pub fn pool_connection<T: ToString>(msg: T) -> Self {
        Self::PoolConnection(msg.to_string())
    }
}
