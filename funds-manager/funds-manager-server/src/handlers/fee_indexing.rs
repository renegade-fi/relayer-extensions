//! Handlers for fee indexing endpoints

use std::sync::Arc;

use bytes::Bytes;
use funds_manager_api::{
    fees::{
        FeeWalletsResponse, UnredeemedFeeTotal, UnredeemedFeeTotalsResponse,
        WithdrawFeeBalanceRequest,
    },
    quoters::DepositAddressResponse,
};
use renegade_common::types::chain::Chain;
use tracing::{error, info};
use warp::reply::Json;

use crate::{custody_client::DepositWithdrawSource, error::ApiError, server::Server};

/// Handler for indexing fees
pub(crate) async fn index_fees_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    if server.is_relayer_disabled(&chain) {
        info!("index_fees: skipping {chain}; relayer disabled via DISABLED_RELAYER_CHAINS");
        return Ok(warp::reply::json(&"Fee indexing skipped (chain relayer disabled)"));
    }
    let indexer = server.get_fee_indexer(&chain)?;
    tokio::task::spawn(async move {
        if let Err(e) = indexer.index_fees().await {
            error!("Error indexing fees: {e}");
        }
    });

    Ok(warp::reply::json(&"Fee indexing initiated"))
}

/// Handler for redeeming fees
pub(crate) async fn redeem_fees_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    if server.is_relayer_disabled(&chain) {
        info!("redeem_fees: skipping {chain}; relayer disabled via DISABLED_RELAYER_CHAINS");
        return Ok(warp::reply::json(&"Fee redemption skipped (chain relayer disabled)"));
    }
    let indexer = server.get_fee_indexer(&chain)?;
    tokio::task::spawn(async move {
        if let Err(e) = indexer.redeem_fees().await {
            error!("Error redeeming fees: {e}");
        }
    });

    Ok(warp::reply::json(&"Fee redemption initiated"))
}

/// Handler for getting fee wallets
pub(crate) async fn get_fee_wallets_handler(
    chain: Chain,
    _body: Bytes, // no body
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    if server.is_relayer_disabled(&chain) {
        // Return an empty wallets list so callers (gardener, etc.) don't see
        // 5xx errors against a wound-down relayer. Both gardener's NAV and
        // holdings paths already treat an empty wallets array as "no fee-
        // redemption inventory on this chain", which is the correct answer.
        return Ok(warp::reply::json(&FeeWalletsResponse { wallets: vec![] }));
    }
    let indexer = server.get_fee_indexer(&chain)?;
    let wallets = indexer.fetch_fee_wallets().await?;

    Ok(warp::reply::json(&FeeWalletsResponse { wallets }))
}

/// Handler for withdrawing a fee balance
pub(crate) async fn withdraw_fee_balance_handler(
    chain: Chain,
    req: WithdrawFeeBalanceRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    if server.is_relayer_disabled(&chain) {
        // Operator-driven action — fail loud rather than silently swallow.
        return Err(warp::reject::custom(ApiError::BadRequest(format!(
            "{chain} relayer is disabled; fee withdrawal cannot proceed"
        ))));
    }
    let indexer = server.get_fee_indexer(&chain)?;
    tokio::task::spawn(async move {
        if let Err(e) = indexer.withdraw_fee_balance(req.wallet_id, req.mint).await {
            error!("Error withdrawing fee balance: {e}");
        }
    });

    Ok(warp::reply::json(&"Fee withdrawal initiated"))
}

/// Handler for retrieving the hot wallet address for fee redemption
pub(crate) async fn get_fee_hot_wallet_address_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    let address = custody_client
        .get_deposit_address(DepositWithdrawSource::FeeRedemption)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for querying the total amount of unredeemed fees for each mint
pub(crate) async fn get_unredeemed_fee_totals_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let indexer = server.get_fee_indexer(&chain)?;
    let totals_vec = indexer.get_unredeemed_fee_totals().await?;
    let totals =
        totals_vec.into_iter().map(|(mint, amount)| UnredeemedFeeTotal { mint, amount }).collect();

    Ok(warp::reply::json(&UnredeemedFeeTotalsResponse { totals }))
}
