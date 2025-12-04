//! HTTP API handlers

use std::sync::Arc;

use darkpool_indexer_api::types::http::{ApiStateObject, BackfillRequest, GetUserStateResponse};
use diesel_async::{AsyncConnection, scoped_futures::ScopedFutureExt};
use tracing::error;
use uuid::Uuid;
use warp::{http::StatusCode, reject::Rejection, reply::Reply};

use crate::{
    api::http::error::ApiError,
    db::{client::DbClient, error::DbError},
    indexer::{Indexer, error::IndexerError},
};

// --------------------
// | Backfill Handler |
// --------------------

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

// --------------------------
// | Get User State Handler |
// --------------------------

/// Handle a request to get a user's state
pub async fn handle_get_user_state_request(
    account_id: Uuid,
    indexer: Arc<Indexer>,
) -> Result<impl Reply, Rejection> {
    let active_state_objects = get_all_active_user_state_objects(account_id, &indexer.db_client)
        .await
        .map_err(ApiError::internal_server_error)?;

    Ok(warp::reply::json(&GetUserStateResponse { active_state_objects }))
}

/// Get all of a user's active state objects
pub async fn get_all_active_user_state_objects(
    account_id: Uuid,
    db_client: &DbClient,
) -> Result<Vec<ApiStateObject>, IndexerError> {
    let mut conn = db_client.get_db_conn().await?;

    let (balances, intents, public_intents) = conn
        .transaction(move |conn| {
            async move {
                let balances = db_client.get_account_active_balances(account_id, conn).await?;

                let intents = db_client.get_account_active_intents(account_id, conn).await?;

                let public_intents =
                    db_client.get_account_active_public_intents(account_id, conn).await?;

                Ok::<_, DbError>((balances, intents, public_intents))
            }
            .scope_boxed()
        })
        .await?;

    let api_balances = balances.into_iter().map(Into::into);
    let api_intents = intents.into_iter().map(Into::into);
    let api_public_intents = public_intents.into_iter().map(Into::into);

    let active_state_objects = api_balances.chain(api_intents).chain(api_public_intents).collect();

    Ok(active_state_objects)
}
