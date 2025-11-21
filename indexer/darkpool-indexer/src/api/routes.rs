//! HTTP route definitions for the darkpool indexer API

use std::sync::Arc;

use darkpool_indexer_api::routes::HEALTHCHECK_PATH;
use warp::{Filter, filters::BoxedFilter, http::StatusCode, reply::Reply};

use crate::indexer::Indexer;

/// Build the routes for the darkpool indexer HTTP server
#[allow(clippy::needless_pass_by_value)]
pub fn http_routes(_indexer: Arc<Indexer>) -> BoxedFilter<(impl Reply,)> {
    let healthcheck_route = warp::get()
        .and(warp::path(HEALTHCHECK_PATH))
        .map(|| warp::reply::with_status("OK", StatusCode::OK));

    healthcheck_route.boxed()
}
