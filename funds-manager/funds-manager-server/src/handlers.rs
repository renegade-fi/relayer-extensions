//! Route handlers for the funds manager

use crate::custody_client::DepositWithdrawSource;
use crate::error::ApiError;
use crate::Server;
use funds_manager_api::{DepositAddressResponse, WithdrawFundsRequest};
use std::collections::HashMap;
use std::sync::Arc;
use warp::reply::Json;

/// The "mint" query param
pub const MINT_QUERY_PARAM: &str = "mint";

/// Handler for indexing fees
pub(crate) async fn index_fees_handler(server: Arc<Server>) -> Result<Json, warp::Rejection> {
    let mut indexer = server
        .build_indexer()
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    indexer
        .index_fees()
        .await
        .map_err(|e| warp::reject::custom(ApiError::IndexingError(e.to_string())))?;
    Ok(warp::reply::json(&"Fees indexed successfully"))
}

/// Handler for redeeming fees
pub(crate) async fn redeem_fees_handler(server: Arc<Server>) -> Result<Json, warp::Rejection> {
    let mut indexer = server
        .build_indexer()
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    indexer
        .redeem_fees()
        .await
        .map_err(|e| warp::reject::custom(ApiError::RedemptionError(e.to_string())))?;
    Ok(warp::reply::json(&"Fees redeemed successfully"))
}

/// Handler for withdrawing funds from custody
pub(crate) async fn quoter_withdraw_handler(
    withdraw_request: WithdrawFundsRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    server
        .custody_client
        .withdraw(
            DepositWithdrawSource::Quoter,
            &withdraw_request.mint,
            withdraw_request.amount,
            withdraw_request.address,
        )
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&"Withdrawal complete"))
}

/// Handler for retrieving the address to deposit custody funds to
pub(crate) async fn get_deposit_address_handler(
    query_params: HashMap<String, String>,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let mint = query_params.get(MINT_QUERY_PARAM).ok_or_else(|| {
        warp::reject::custom(ApiError::BadRequest("Missing 'mint' query parameter".to_string()))
    })?;

    let address = server
        .custody_client
        .get_deposit_address(mint, DepositWithdrawSource::Quoter)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}
