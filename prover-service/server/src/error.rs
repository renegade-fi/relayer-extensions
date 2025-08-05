//! Error types emitted by the service

use http::StatusCode;
use serde_json::json;
use warp::reply::{Json, WithStatus};

/// The error type for the service
#[derive(Debug, thiserror::Error)]
pub enum ProverServiceError {
    /// A custom error
    #[error("error: {0}")]
    Custom(String),
    /// An error proving a circuit
    #[error("error proving circuit: {0}")]
    Prover(String),
    /// The error occurred while setting up telemetry
    #[error("error setting up server: {0}")]
    Setup(String),
}

impl warp::reject::Reject for ProverServiceError {}

impl ProverServiceError {
    // --- Constructors --- //

    /// Create a new custom error
    #[allow(clippy::needless_pass_by_value)]
    pub fn custom<T: ToString>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }

    /// Create a new prover error
    #[allow(clippy::needless_pass_by_value)]
    pub fn prover<T: ToString>(msg: T) -> Self {
        Self::Prover(msg.to_string())
    }

    /// Create a new setup error
    #[allow(clippy::needless_pass_by_value)]
    pub fn setup<T: ToString>(msg: T) -> Self {
        Self::Setup(msg.to_string())
    }

    // --- Reply -- //

    /// Convert a prover service error to a warp reply
    pub fn to_reply(&self) -> WithStatus<Json> {
        #[allow(clippy::match_single_binding)]
        let (code, msg) = match self {
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
