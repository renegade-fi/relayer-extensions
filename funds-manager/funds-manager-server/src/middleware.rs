//! Middleware for the funds manager server

use crate::error::ApiError;
use crate::Server;
use bytes::Bytes;
use funds_manager_api::auth::{get_request_bytes, X_SIGNATURE_HEADER};
use serde::de::DeserializeOwned;
use std::sync::Arc;
use warp::Filter;

/// Add HMAC authentication to a route
pub(crate) fn with_hmac_auth(
    server: Arc<Server>,
) -> impl Filter<Extract = (Bytes,), Error = warp::Rejection> + Clone {
    warp::any()
        .and(warp::any().map(move || server.clone()))
        .and(warp::header::optional::<String>(X_SIGNATURE_HEADER))
        .and(warp::method())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and_then(verify_hmac)
}

/// Verify the HMAC signature
async fn verify_hmac(
    server: Arc<Server>,
    signature: Option<String>,
    method: warp::http::Method,
    path: warp::path::FullPath,
    headers: warp::http::HeaderMap,
    body: Bytes,
) -> Result<Bytes, warp::Rejection> {
    // Unwrap the key and signature
    let hmac_key = match &server.hmac_key {
        Some(hmac_key) => hmac_key,
        None => return Ok(body), // Auth is disabled, allow the request
    };

    let signature = match signature {
        Some(sig) => sig,
        None => {
            return Err(warp::reject::custom(ApiError::Unauthenticated(
                "Missing signature".to_string(),
            )))
        },
    };

    let expected = get_request_bytes(method.as_str(), path.as_str(), &headers, &body);
    let provided = hex::decode(&signature)
        .map_err(|_| warp::reject::custom(ApiError::BadRequest("Invalid signature".to_string())))?;
    if !hmac_key.verify_mac(&expected, &provided) {
        return Err(warp::reject::custom(ApiError::Unauthenticated(
            "Invalid signature".to_string(),
        )));
    }

    Ok(body)
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
