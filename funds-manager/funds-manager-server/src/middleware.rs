//! Middleware for the funds manager server

use crate::error::ApiError;
use crate::Server;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use itertools::Itertools;
use serde::de::DeserializeOwned;
use sha2::Sha256;
use std::sync::Arc;
use warp::Filter;

/// The header key for the HMAC signature
const X_SIGNATURE_HEADER: &str = "X-Signature";
/// The prefix for Renegade headers, these headers are included in the HMAC
/// signature
const RENEGADE_HEADER_PREFIX: &str = "x-renegade-";

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

    // Construct the MAC
    let mut mac = Hmac::<Sha256>::new_from_slice(hmac_key)
        .map_err(|_| warp::reject::custom(ApiError::InternalError("HMAC error".to_string())))?;

    // Update with method, path, headers and body in order
    mac.update(method.as_str().as_bytes());
    mac.update(path.as_str().as_bytes());
    add_headers_to_hmac(&mut mac, &headers);
    mac.update(&body);

    // Check the signature
    let expected = mac.finalize().into_bytes();
    let provided = hex::decode(signature)
        .map_err(|_| warp::reject::custom(ApiError::BadRequest("Invalid signature".to_string())))?;
    if expected.as_slice() != provided.as_slice() {
        return Err(warp::reject::custom(ApiError::Unauthenticated(
            "Invalid signature".to_string(),
        )));
    }

    Ok(body)
}

/// Hash headers into an HMAC
fn add_headers_to_hmac(mac: &mut Hmac<Sha256>, headers: &warp::http::HeaderMap) {
    let mut renegade_headers = headers
        .iter()
        .filter_map(|(k, v)| {
            let key = k.as_str().to_lowercase();
            if key.starts_with(RENEGADE_HEADER_PREFIX) {
                Some((key, v.to_str().unwrap_or("").to_string()))
            } else {
                None
            }
        })
        .collect_vec();
    renegade_headers.sort_by(|a, b| a.0.cmp(&b.0));

    for (key, value) in renegade_headers {
        mac.update(key.as_bytes());
        mac.update(value.as_bytes());
    }
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
