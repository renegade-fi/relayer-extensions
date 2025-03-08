//! General-purpose HTTP utilities

use std::time::Duration;

use reqwest::{Client, Response};
use serde::Serialize;

use thiserror::Error;

/// An error with the HTTP client
#[derive(Debug, Error)]
pub enum HttpError {
    /// A network error
    #[error("Network error: {0}")]
    Network(#[source] reqwest::Error),

    /// API error
    #[error("API error: {0}")]
    Api(String),

    /// Parsing error
    #[error("Parsing error: {0}")]
    Parsing(String),
}

impl HttpError {
    /// Create a new parsing error
    #[allow(clippy::needless_pass_by_value)]
    pub fn parsing<T: ToString>(msg: T) -> Self {
        Self::Parsing(msg.to_string())
    }
}

/// Sends a basic GET request
pub async fn send_get_request(url: &str, timeout_secs: u64) -> Result<Response, HttpError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(HttpError::Network)?;

    let response = client.get(url).send().await.map_err(HttpError::Network)?;

    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.map_err(HttpError::parsing)?;

        return Err(HttpError::Api(format!("Status {}: {}", status, message)));
    }

    Ok(response)
}

/// Sends a basic POST request
pub async fn send_post_request<T: Serialize>(
    url: &str,
    body: Option<T>,
    timeout_secs: u64,
) -> Result<Response, HttpError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(HttpError::Network)?;

    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(HttpError::Network)?;

    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.map_err(HttpError::parsing)?;
        return Err(HttpError::Api(format!("Status {}: {}", status, message)));
    }

    Ok(response)
}
