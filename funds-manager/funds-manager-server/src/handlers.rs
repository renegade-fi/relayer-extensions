//! Route handlers for the funds manager

use crate::custody_client::DepositWithdrawSource;
use crate::error::ApiError;
use crate::Server;
use bytes::Bytes;
use funds_manager_api::{
    CreateHotWalletRequest, CreateHotWalletResponse, DepositAddressResponse, FeeWalletsResponse,
    HotWalletBalancesResponse, WithdrawFeeBalanceRequest, WithdrawFundsRequest, WithdrawGasRequest,
};
use itertools::Itertools;
use std::collections::HashMap;
use std::sync::Arc;
use warp::reply::Json;

/// The "mints" query param
pub const MINTS_QUERY_PARAM: &str = "mints";
/// The asset used for gas (ETH)
pub const GAS_ASSET_NAME: &str = "ETH";
/// The maximum amount of gas that can be withdrawn at a given time
pub const MAX_GAS_WITHDRAWAL_AMOUNT: f64 = 0.1; // 0.1 ETH

/// Handler for indexing fees
pub(crate) async fn index_fees_handler(server: Arc<Server>) -> Result<Json, warp::Rejection> {
    let mut indexer = server
        .build_indexer()
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
        .withdraw_with_token_addr(
            DepositWithdrawSource::Quoter,
            &withdraw_request.address,
            &withdraw_request.mint,
            withdraw_request.amount,
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
    let mint = query_params.get(MINTS_QUERY_PARAM).ok_or_else(|| {
        warp::reject::custom(ApiError::BadRequest("Missing 'mints' query parameter".to_string()))
    })?;

    let address = server
        .custody_client
        .get_deposit_address(mint, DepositWithdrawSource::Quoter)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for withdrawing gas from custody
pub(crate) async fn withdraw_gas_handler(
    withdraw_request: WithdrawGasRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    if withdraw_request.amount > MAX_GAS_WITHDRAWAL_AMOUNT {
        return Err(warp::reject::custom(ApiError::BadRequest(format!(
            "Requested amount {} ETH exceeds maximum allowed withdrawal of {} ETH",
            withdraw_request.amount, MAX_GAS_WITHDRAWAL_AMOUNT
        ))));
    }

    server
        .custody_client
        .withdraw(
            DepositWithdrawSource::Gas,
            &withdraw_request.destination_address,
            GAS_ASSET_NAME,
            withdraw_request.amount,
        )
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&format!("Gas withdrawal of {} ETH complete", withdraw_request.amount)))
}

/// Handler for getting fee wallets
pub(crate) async fn get_fee_wallets_handler(
    _body: Bytes, // no body
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let mut indexer = server.build_indexer()?;
    let wallets = indexer.fetch_fee_wallets().await?;
    Ok(warp::reply::json(&FeeWalletsResponse { wallets }))
}

/// Handler for withdrawing a fee balance
pub(crate) async fn withdraw_fee_balance_handler(
    req: WithdrawFeeBalanceRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let mut indexer = server.build_indexer()?;
    indexer
        .withdraw_fee_balance(req.wallet_id, req.mint)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&"Fee withdrawal initiated..."))
}

/// Handler for creating a hot wallet
pub(crate) async fn create_hot_wallet_handler(
    req: CreateHotWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let address = server
        .custody_client
        .create_hot_wallet(req.vault)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = CreateHotWalletResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for getting hot wallet balances
pub(crate) async fn get_hot_wallet_balances_handler(
    _body: Bytes, // unused
    query_params: HashMap<String, String>,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let mints = query_params
        .get(MINTS_QUERY_PARAM)
        .map(|s| s.split(',').map(String::from).collect_vec())
        .unwrap_or_default();

    let wallets = server
        .custody_client
        .get_hot_wallet_balances(&mints)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = HotWalletBalancesResponse { wallets };
    Ok(warp::reply::json(&resp))
}
