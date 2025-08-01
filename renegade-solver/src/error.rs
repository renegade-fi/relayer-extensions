//! Error types for the solver

use alloy::primitives::U256;
use price_reporter_client::error::PriceReporterClientError;
use renegade_sdk::ExternalMatchClientError;
use serde_json::json;
use thiserror::Error;
use warp::{
    http::StatusCode,
    reject::Reject,
    reply::{Json, WithStatus},
    Rejection,
};

use crate::uniswapx::fixed_point::error::FixedPointMathError;

/// Type alias for Results using SolverError
pub type SolverResult<T> = Result<T, SolverError>;

/// The generic solver error
#[derive(Error, Debug)]
pub enum SolverError {
    /// An error ABI encoding/decoding a value
    #[error("ABI encoding/decoding error: {0}")]
    AbiEncoding(String),
    /// HTTP error occurred
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// An invalid u256 was provided
    #[error("Invalid u256: {0}")]
    InvalidU256(U256),
    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Error from the renegade client
    #[error("Renegade client error: {0}")]
    Renegade(#[from] ExternalMatchClientError),
    /// Error from the price reporter client
    #[error("Price reporter client error: {0}")]
    PriceReporter(#[from] PriceReporterClientError),
    /// Fixed point math error
    #[error("Fixed point math error: {0}")]
    FixedPoint(#[from] FixedPointMathError),
    /// An order's input and outputs both scale with priority fee
    #[error("Input and outputs both scale with priority fee")]
    InputOutputScaling,
}

impl SolverError {
    /// Create an ABI encoding/decoding error
    #[allow(clippy::needless_pass_by_value)]
    pub fn abi_encoding<S: ToString>(msg: S) -> Self {
        Self::AbiEncoding(msg.to_string())
    }
}

impl Reject for SolverError {}

// ------------------
// | Error Handling |
// ------------------

/// Handle rejections and convert SolverError to JSON responses
pub async fn handle_rejection(err: Rejection) -> Result<WithStatus<Json>, Rejection> {
    if let Some(solver_error) = err.find::<SolverError>() {
        #[allow(clippy::match_single_binding)]
        let (status_code, message) = match solver_error {
            _ => {
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
