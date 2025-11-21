//! HTTP route definitions for the darkpool indexer API

use std::sync::Arc;

use darkpool_indexer_api::{
    routes::{BACKFILL_PATH, GET_USER_STATE_PATH, HEALTHCHECK_PATH},
    types::http::BackfillRequest,
};
use uuid::Uuid;
use warp::{Filter, Rejection, Reply, filters::BoxedFilter, http::StatusCode};

use crate::{
    api::http::{
        handlers::{handle_backfill_request, handle_get_user_state_request},
        middleware::{handle_rejection, identity, with_hmac_auth, with_indexer, with_json_body},
    },
    indexer::Indexer,
};

/// Build the routes for the darkpool indexer HTTP server
#[allow(clippy::needless_pass_by_value)]
pub fn http_routes(indexer: Arc<Indexer>) -> BoxedFilter<(impl Reply,)> {
    let healthcheck_route = warp::get()
        .and(warp::path(HEALTHCHECK_PATH))
        .map(|| warp::reply::with_status("OK", StatusCode::OK));

    let backfill_route = warp::post()
        .and(warp::path(BACKFILL_PATH))
        .and(with_hmac_auth(indexer.clone()))
        .map(with_json_body::<BackfillRequest>)
        .and_then(identity)
        .and(with_indexer(indexer.clone()))
        .and_then(handle_backfill_request);

    let user_state_route = warp::get()
        .and(warp::path(GET_USER_STATE_PATH))
        .and(warp::path::param::<Uuid>())
        .and(with_hmac_auth(indexer.clone()))
        .and(with_indexer(indexer))
        .and_then(|account_id, _, indexer| handle_get_user_state_request(account_id, indexer));

    let not_found_fallback = warp::any().and_then(|| async {
        Ok::<_, Rejection>(warp::reply::with_status("Not Found", StatusCode::NOT_FOUND))
    });

    healthcheck_route
        .or(backfill_route)
        .or(user_state_route)
        .or(not_found_fallback)
        .recover(handle_rejection)
        .boxed()
}
