//! Request and response utilities

use std::time::Duration;

use bytes::Bytes;
use http::header::CONTENT_LENGTH;
use reqwest::{Client, Response};
use serde::Serialize;
use serde_json::json;
use thiserror::Error;
use warp::http::Response as HttpResponse;
use warp::reply::Reply;

use crate::error::AuthServerError;
use crate::http_utils::stringify_formatter::json_serialize;

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

/// Convert a `warp::hyper::HeaderMap` (using the old `http` crate version)
/// into a fresh `http::HeaderMap` that comes from the new 1.x `http` crate.
///
/// We avoid the `IntoHeaderName` trait conflict that arises when two
/// different `http` crate versions are in the dependency graph by copying
/// header names/values byte‐for‐byte into the new types.
pub fn convert_headers(headers: &warp::hyper::HeaderMap) -> http1::HeaderMap {
    let mut converted = http1::HeaderMap::new();
    for (name, value) in headers.iter() {
        let name_bytes = name.as_ref();
        let name = match http1::header::HeaderName::from_bytes(name_bytes) {
            Ok(n) => n,
            Err(_) => continue, // Skip invalid names
        };

        let value_bytes = value.as_ref();
        let value = match http1::HeaderValue::from_bytes(value_bytes) {
            Ok(v) => v,
            Err(_) => continue, // Skip invalid values
        };
        converted.append(name, value);
    }

    converted
}

// ------------
// | Requests |
// ------------

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
    stringify: bool,
) -> Result<(), AuthServerError> {
    let serialized = json_serialize(&body, stringify)?;
    let body_bytes = Bytes::from(serialized);

    resp.headers_mut().insert(CONTENT_LENGTH, body_bytes.len().into());
    *resp.body_mut() = body_bytes;

    Ok(())
}
