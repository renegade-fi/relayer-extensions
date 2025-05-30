//! General-purpose HTTP utilities

use std::time::Duration;

use bytes::Bytes;
use http::{header::CONTENT_LENGTH, Response as HttpResponse};
use reqwest::{Client, Response};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use warp::reply::Reply;

use crate::error::AuthServerError;

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

// ------------
// | Requests |
// ------------

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

// ------------------
// | Response Utils |
// ------------------

/// Construct empty json reply
pub fn empty_json_reply() -> impl Reply {
    warp::reply::json(&json!({}))
}

/// Overwrite the body of an HTTP response
pub fn overwrite_response_body<T: Serialize>(
    resp: &mut HttpResponse<Bytes>,
    body: T,
) -> Result<(), AuthServerError> {
    let body_bytes = Bytes::from(serde_json::to_vec(&body).map_err(AuthServerError::serde)?);

    resp.headers_mut().insert(CONTENT_LENGTH, body_bytes.len().into());
    *resp.body_mut() = body_bytes;

    Ok(())
}
