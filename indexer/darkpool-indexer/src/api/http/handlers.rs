//! HTTP API handlers

use std::sync::Arc;

use darkpool_indexer_api::types::http::BackfillRequest;
use tracing::error;
use warp::{http::StatusCode, reject::Rejection, reply::Reply};

use crate::indexer::Indexer;

/// Handle a request to backfill a user's state
pub async fn handle_backfill_request(
    req: BackfillRequest,
    indexer: Arc<Indexer>,
) -> Result<impl Reply, Rejection> {
    let BackfillRequest { account_id } = req;

    // Spawn a background task to backfill the user's state so that we can send an
    // API response immediately
    tokio::spawn(async move {
        if let Err(e) = indexer.backfill_user_state(account_id).await {
            error!("Error backfilling state for account {account_id}: {e}");
        }
    });

    Ok(warp::reply::with_status("OK", StatusCode::OK))
}
