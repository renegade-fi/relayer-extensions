//! Middleware for the darkpool indexer API

use std::{convert::Infallible, sync::Arc};

use alloy::transports::http::reqwest::StatusCode;
use bytes::Bytes;
use serde::de::DeserializeOwned;
use tracing::{error, warn};
use warp::{
    Filter, filters::path::FullPath, http::HeaderMap, reject::Rejection, reply::WithStatus,
};

use crate::{
    api::http::{error::ApiError, relayer_auth_helpers::validate_expiring_auth},
    indexer::Indexer,
};

// ---------
// | Types |
// ---------

/// A trait alias for a warp filter that extracts the given type,
/// erroring with `warp::Rejection` if the extraction fails
pub trait FilterExtracts<T> = Filter<Extract = T, Error = Rejection> + Clone + Send;

/// /// A trait alias for a warp filter that extracts the given type infallibly
pub trait FilterExtractsInfallible<T> = Filter<Extract = T, Error = Infallible> + Clone;

// --------------
// | Middleware |
// --------------

/// Helper function to clone & pass the indexer to filters
pub fn with_indexer(indexer: Arc<Indexer>) -> impl FilterExtractsInfallible<(Arc<Indexer>,)> {
    warp::any().map(move || indexer.clone())
}

/// Extract a JSON body from a request
#[allow(clippy::needless_pass_by_value)]
pub fn with_json_body<T: DeserializeOwned + Send>(body: Bytes) -> Result<T, Rejection> {
    serde_json::from_slice(&body).map_err(|e| {
        warp::reject::custom(ApiError::bad_request(format!("Invalid JSON request body: {e}")))
    })
}

/// Identity map for a handler's middleware, used to chain together `map`s and
/// `and_then`s
pub async fn identity<T>(res: T) -> T {
    res
}

/// Add HMAC authentication to a route
pub fn with_hmac_auth(indexer: Arc<Indexer>) -> impl FilterExtracts<(Bytes,)> {
    warp::any()
        .and(with_indexer(indexer))
        .and(with_hmac_inputs())
        .and_then(verify_hmac_auth)
        .untuple_one()
}

/// Extract the path, headers, and body from the request for use in HMAC
/// authentication
fn with_hmac_inputs() -> impl FilterExtracts<(FullPath, HeaderMap, Bytes)> {
    warp::any().and(warp::path::full()).and(warp::header::headers_cloned()).and(warp::body::bytes())
}

/// Handle a rejection from an endpoint handler
pub async fn handle_rejection(err: Rejection) -> Result<WithStatus<String>, Rejection> {
    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message) = match api_error {
            ApiError::Unauthorized(e) => (StatusCode::UNAUTHORIZED, e.clone()),
            ApiError::BadRequest(e) => (StatusCode::BAD_REQUEST, e.clone()),
        };

        Ok(warp::reply::with_status(message, code))
    } else {
        error!("unhandled rejection: {:?}", err);
        Err(err)
    }
}

// -----------
// | Helpers |
// -----------

/// Verify the request using the indexer's auth key, and return the body for
/// subsequent usage
async fn verify_hmac_auth(
    indexer: Arc<Indexer>,
    path: FullPath,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(Bytes,), Rejection> {
    match &indexer.http_auth_key {
        Some(hmac_key) => {
            validate_expiring_auth(path.as_str(), &headers, &body, &hmac_key)
                .map_err(ApiError::unauthorized)?;
        },
        None => {
            warn!("HMAC authentication disabled for HTTP API, allowing request");
        },
    }

    Ok((body,))
}
