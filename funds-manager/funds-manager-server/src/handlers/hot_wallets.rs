//! Handlers for hot wallet endpoints

use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use funds_manager_api::hot_wallets::{
    CreateHotWalletRequest, CreateHotWalletResponse, HotWalletBalancesResponse,
};
use itertools::Itertools;
use renegade_types_core::Chain;
use warp::reply::Json;

use crate::{error::ApiError, server::Server};

// -------------
// | Constants |
// -------------

/// The "mints" query param
const MINTS_QUERY_PARAM: &str = "mints";

/// Handler for creating a hot wallet
pub(crate) async fn create_hot_wallet_handler(
    chain: Chain,
    req: CreateHotWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;

    let address = custody_client
        .create_hot_wallet(req.vault, req.internal_wallet_id)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = CreateHotWalletResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for getting hot wallet balances
pub(crate) async fn get_hot_wallet_balances_handler(
    chain: Chain,
    _body: Bytes, // unused
    query_params: HashMap<String, String>,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;

    let mints = query_params
        .get(MINTS_QUERY_PARAM)
        .map(|s| s.split(',').map(String::from).collect_vec())
        .unwrap_or_default();

    let wallets = custody_client
        .get_hot_wallet_balances(&mints)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = HotWalletBalancesResponse { wallets };
    Ok(warp::reply::json(&resp))
}
