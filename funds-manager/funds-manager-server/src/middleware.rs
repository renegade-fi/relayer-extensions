//! Middleware for the funds manager server

use crate::error::ApiError;
use crate::helpers::convert_headers;
use crate::{with_server, Server};
use bytes::Bytes;
use funds_manager_api::auth::{get_request_bytes, X_SIGNATURE_HEADER};
use http::{HeaderMap, Method};
use renegade_api::auth::validate_expiring_auth;
use renegade_common::types::chain::Chain;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use warp::filters::path::FullPath;
use warp::Filter;

// ---------
// | Types |
// ---------

/// A trait alias for a warp filter that extracts the given type,
/// erroring with `warp::Rejection` if the extraction fails
pub trait FilterExtracts<T> = Filter<Extract = T, Error = warp::Rejection> + Clone + Send;

/// Add HMAC authentication to a route
pub(crate) fn with_hmac_auth(server: Arc<Server>) -> impl FilterExtracts<(Bytes,)> {
    warp::any().and(with_server(server)).and(with_hmac_inputs()).and_then(verify_hmac).untuple_one()
}

/// Extract the path, headers, and body from the request
/// for use in HMAC authentication
fn with_hmac_inputs() -> impl FilterExtracts<(String, Option<String>, Method, HeaderMap, Bytes)> {
    warp::any()
        .and(warp::path::full())
        .and(with_query_string())
        .and_then(|path: FullPath, query: String| async move {
            let path_str = if query.is_empty() {
                path.as_str().to_string()
            } else {
                format!("{}?{}", path.as_str(), query)
            };
            Ok::<_, warp::Rejection>(path_str)
        })
        .and(warp::header::optional::<String>(X_SIGNATURE_HEADER))
        .and(warp::method())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
}

/// Extract the query string from the request, or an empty string if one is not
/// present
pub(crate) fn with_query_string() -> impl FilterExtracts<(String,)> {
    warp::query::raw()
        .or_else(|_| async move { Ok::<(String,), warp::Rejection>(("".to_string(),)) })
}

/// Verify the HMAC signature
async fn verify_hmac(
    server: Arc<Server>,
    path: String,
    signature: Option<String>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(Bytes,), warp::Rejection> {
    // Unwrap the key
    let hmac_key = match &server.hmac_key {
        Some(hmac_key) => hmac_key,
        None => return Ok((body,)), // Auth is disabled, allow the request
    };

    let auth_headers = convert_headers(&headers);
    // Try v2 auth first
    if validate_expiring_auth(&path, &auth_headers, &body, hmac_key).is_ok() {
        return Ok((body,));
    }

    // Fall back to v1 auth

    // Unwrap the signature
    let signature = match signature {
        Some(sig) => sig,
        None => {
            return Err(warp::reject::custom(ApiError::Unauthenticated(
                "Missing signature".to_string(),
            )))
        },
    };

    let path = path.split("?").next().unwrap_or(&path);

    let expected = get_request_bytes(method.as_str(), path, &headers, &body);
    let provided = hex::decode(&signature)
        .map_err(|_| warp::reject::custom(ApiError::BadRequest("Invalid signature".to_string())))?;
    if !hmac_key.verify_mac(&expected, &provided) {
        return Err(warp::reject::custom(ApiError::Unauthenticated(
            "Invalid signature".to_string(),
        )));
    }

    Ok((body,))
}

/// Extract a JSON body from a request
#[allow(clippy::needless_pass_by_value)]
pub fn with_chain_and_json_body<T: DeserializeOwned + Send>(
    chain: Chain,
    body: Bytes,
) -> Result<(Chain, T), warp::Rejection> {
    with_json_body(body).map(|body| (chain, body))
}

/// Extract a JSON body from a request
#[allow(clippy::needless_pass_by_value)]
pub fn with_json_body<T: DeserializeOwned + Send>(body: Bytes) -> Result<T, warp::Rejection> {
    serde_json::from_slice(&body)
        .map_err(|e| warp::reject::custom(ApiError::BadRequest(format!("Invalid JSON: {}", e))))
}

/// Identity map for a handler's middleware, used to chain together `map`s and
/// `and_then`s
pub async fn identity<T>(res: T) -> T {
    res
}
