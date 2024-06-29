//! Error types for the compliance server

use std::{error::Error, fmt::Display};

use warp::reject::Reject;

/// The error type emitted by the compliance server
#[derive(Debug, Clone)]
pub enum ComplianceServerError {
    /// An error with a database query
    Db(String),
    /// An error with the chainalysis API
    Chainalysis(String),
}

impl Display for ComplianceServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComplianceServerError::Db(e) => write!(f, "Database error: {}", e),
            ComplianceServerError::Chainalysis(e) => write!(f, "Chainalysis error: {}", e),
        }
    }
}
impl Error for ComplianceServerError {}
impl Reject for ComplianceServerError {}

impl From<reqwest::Error> for ComplianceServerError {
    fn from(e: reqwest::Error) -> Self {
        ComplianceServerError::Chainalysis(e.to_string())
    }
}
