//! HTTP API error definitions

use warp::reject::Reject;

/// HTTP API errors
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// An unauthorized error
    #[error("unauthorized: {0}")]
    Unauthorized(String),
}

impl Reject for ApiError {}

impl ApiError {
    /// Create a new `Unauthorized` error
    #[allow(clippy::needless_pass_by_value)]
    pub fn unauthorized<T: ToString>(e: T) -> Self {
        Self::Unauthorized(e.to_string())
    }
}
