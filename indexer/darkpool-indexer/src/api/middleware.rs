//! Middleware for the darkpool indexer API

use std::{convert::Infallible, sync::Arc};

use bytes::Bytes;
use tracing::warn;
use warp::{Filter, filters::path::FullPath, http::HeaderMap, reject::Rejection};

use crate::{
    api::{error::ApiError, relayer_auth_helpers::validate_expiring_auth},
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

/// Add HMAC authentication to a route
fn with_hmac_auth(indexer: Arc<Indexer>) -> impl FilterExtracts<(Bytes,)> {
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
