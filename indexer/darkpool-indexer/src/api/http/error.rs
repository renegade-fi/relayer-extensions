//! HTTP API error definitions

use warp::reject::Reject;

/// HTTP API errors
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// An unauthorized error
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// A bad request error
    #[error("bad request: {0}")]
    BadRequest(String),
    /// An internal server error
    #[error("internal server error: {0}")]
    InternalServerError(String),
}

impl Reject for ApiError {}

#[allow(clippy::needless_pass_by_value)]
impl ApiError {
    /// Create a new `Unauthorized` error
    pub fn unauthorized<T: ToString>(e: T) -> Self {
        Self::Unauthorized(e.to_string())
    }

    /// Create a new `BadRequest` error
    pub fn bad_request<T: ToString>(e: T) -> Self {
        Self::BadRequest(e.to_string())
    }

    /// Create a new `InternalServerError` error
    pub fn internal_server_error<T: ToString>(e: T) -> Self {
        Self::InternalServerError(e.to_string())
    }
}
