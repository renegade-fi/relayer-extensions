//! Error types emitted by the service

use http::StatusCode;
use serde_json::json;
use warp::reply::{Json, WithStatus};

/// The error type for the service
#[derive(Debug, thiserror::Error)]
pub enum ProverServiceError {
    /// The request was invalid
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// The error occurred while setting up telemetry
    #[error("error setting up server: {0}")]
    Setup(String),
}

impl warp::reject::Reject for ProverServiceError {}

impl ProverServiceError {
    // --- Constructors --- //

    /// Create a new invalid request error
    #[allow(clippy::needless_pass_by_value)]
    pub fn invalid_request<T: ToString>(msg: T) -> Self {
        Self::InvalidRequest(msg.to_string())
    }

    /// Create a new setup error
    #[allow(clippy::needless_pass_by_value)]
    pub fn setup<T: ToString>(msg: T) -> Self {
        Self::Setup(msg.to_string())
    }

    // --- Reply -- //

    /// Convert a prover service error to a warp reply
    pub fn to_reply(&self) -> WithStatus<Json> {
        let (code, msg) = match self {
            Self::InvalidRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            x => (StatusCode::INTERNAL_SERVER_ERROR, x.to_string()),
        };

        json_error(&msg, code)
    }
}

/// Return a json error from a string message
pub(crate) fn json_error(msg: &str, code: StatusCode) -> WithStatus<Json> {
    let json = json!({ "error": msg });
    warp::reply::with_status(warp::reply::json(&json), code)
}
