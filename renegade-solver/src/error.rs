//! Error types for the solver

use renegade_sdk::ExternalMatchClientError;
use serde_json::json;
use thiserror::Error;
use warp::{
    http::StatusCode,
    reject::Reject,
    reply::{Json, WithStatus},
    Rejection,
};

/// Type alias for Results using SolverError
pub type SolverResult<T> = Result<T, SolverError>;

/// The generic solver error
#[derive(Error, Debug)]
pub enum SolverError {
    /// HTTP error occurred
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Error from the renegade client
    #[error("Renegade client error: {0}")]
    Renegade(#[from] ExternalMatchClientError),
}

impl Reject for SolverError {}

// ------------------
// | Error Handling |
// ------------------

/// Handle rejections and convert SolverError to JSON responses
pub async fn handle_rejection(err: Rejection) -> Result<WithStatus<Json>, Rejection> {
    if let Some(solver_error) = err.find::<SolverError>() {
        let (status_code, message) = match solver_error {
            SolverError::Http(_) | SolverError::Serialization(_) | SolverError::Renegade(_) => {
                let msg = format!("Internal server error: {solver_error}");
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            },
        };

        Ok(json_error(&message, status_code))
    } else {
        Err(err)
    }
}

/// Return a json error from a string message
fn json_error(msg: &str, code: StatusCode) -> WithStatus<Json> {
    let json = json!({ "error": msg });
    warp::reply::with_status(warp::reply::json(&json), code)
}
