//! Handlers for gas endpoints

use std::sync::Arc;

use bytes::Bytes;
use funds_manager_api::{
    gas::{
        CreateGasWalletResponse, GasWalletsResponse, RefillGasRequest, RegisterGasWalletRequest,
        RegisterGasWalletResponse, ReportActivePeersRequest, WithdrawGasRequest,
    },
    quoters::DepositAddressResponse,
};
use renegade_common::types::chain::Chain;
use serde_json::json;
use tracing::{error, warn};
use warp::reply::Json;

use crate::{custody_client::DepositWithdrawSource, error::ApiError, server::Server};

// -------------
// | Constants |
// -------------

/// The maximum amount of gas that can be withdrawn at a given time
const MAX_GAS_WITHDRAWAL_AMOUNT: f64 = 1.; // ETH
/// The maximum amount that a request may refill gas to
const MAX_GAS_REFILL_AMOUNT: f64 = 0.1; // ETH

/// Handler for withdrawing gas from custody
pub(crate) async fn withdraw_gas_handler(
    chain: Chain,
    withdraw_request: WithdrawGasRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    if withdraw_request.amount > MAX_GAS_WITHDRAWAL_AMOUNT {
        return Err(warp::reject::custom(ApiError::BadRequest(format!(
            "Requested amount {} ETH exceeds maximum allowed withdrawal of {} ETH",
            withdraw_request.amount, MAX_GAS_WITHDRAWAL_AMOUNT
        ))));
    }

    let custody_client = server.get_custody_client(&chain)?;
    custody_client
        .withdraw_gas(withdraw_request.amount, &withdraw_request.destination_address)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&"Withdrawal complete"))
}

/// Handler for refilling gas for all active wallets
pub(crate) async fn refill_gas_handler(
    chain: Chain,
    req: RefillGasRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // Check that the refill amount is less than the max
    if req.amount > MAX_GAS_REFILL_AMOUNT {
        return Err(warp::reject::custom(ApiError::BadRequest(format!(
            "Requested amount {} ETH exceeds maximum allowed refill of {} ETH",
            req.amount, MAX_GAS_REFILL_AMOUNT
        ))));
    }

    let custody_client = server.get_custody_client(&chain)?;
    custody_client.refill_gas_wallets(req.amount).await?;

    let resp = json!({});
    Ok(warp::reply::json(&resp))
}

/// Handler for creating a new gas wallet
pub(crate) async fn create_gas_wallet_handler(
    chain: Chain,
    _body: Bytes, // no body
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;

    let address = custody_client
        .create_gas_wallet()
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = CreateGasWalletResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for registering a gas wallet for a peer
pub(crate) async fn register_gas_wallet_handler(
    chain: Chain,
    req: RegisterGasWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;

    let key = custody_client
        .register_gas_wallet(&req.peer_id)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = RegisterGasWalletResponse { key };
    Ok(warp::reply::json(&resp))
}

/// Handler for reporting active peers
pub(crate) async fn report_active_peers_handler(
    chain: Chain,
    req: ReportActivePeersRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    custody_client
        .record_active_gas_wallet(req.peers)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = json!({});
    Ok(warp::reply::json(&resp))
}

/// Handler for refilling gas for the gas sponsor contract
pub(crate) async fn refill_gas_sponsor_handler(
    chain: Chain,
    _body: Bytes, // no body
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    let execution_client = server.get_execution_client(&chain)?;
    let metrics_recorder = server.get_metrics_recorder(&chain)?;

    // Get the quoter hot wallet's private key
    let quoter_wallet = custody_client.get_quoter_hot_wallet().await?;
    let signer = custody_client.get_hot_wallet_private_key(&quoter_wallet.address).await?;

    // Refill the gas sponsor with native ETH
    custody_client.refill_gas_sponsor_eth().await?;

    // Refill the gas sponsor with ERC20s
    let tokens_needing_refill = custody_client.get_tokens_needing_refill().await?;

    // Swap into the target tokens such that we can cover the refill amounts
    let swap_outcomes =
        execution_client.multi_swap_into_target_tokens(&tokens_needing_refill).await?;

    // Send the tokens to the gas sponsor
    for (token, refill_amount) in tokens_needing_refill {
        let ticker = token.get_ticker().unwrap_or(token.get_addr());
        if let Err(e) =
            custody_client.send_token_to_gas_sponsor(&token, refill_amount, signer.clone()).await
        {
            error!("Failed to send {ticker} to gas sponsor, skipping: {e}");
        }
    }

    for outcome in swap_outcomes {
        if let Err(e) = metrics_recorder.record_swap_cost(&outcome).await {
            warn!("Failed to record swap cost metrics: {e}");
        }
    }

    let resp = json!({});
    Ok(warp::reply::json(&resp))
}

/// Handler for getting all gas wallet addresses
pub(crate) async fn get_gas_wallets_handler(
    chain: Chain,
    _body: Bytes, // no body
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    let gas_wallets = custody_client.get_all_gas_wallets().await?;

    let addresses = gas_wallets.iter().map(|wallet| wallet.address.clone()).collect();
    let entries = gas_wallets.into_iter().map(|wallet| wallet.into()).collect();
    let resp = GasWalletsResponse { addresses, entries };

    Ok(warp::reply::json(&resp))
}

/// Handler for retrieving the hot wallet address for gas operations
pub(crate) async fn get_gas_hot_wallet_address_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    let address = custody_client
        .get_deposit_address(DepositWithdrawSource::Gas)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}
