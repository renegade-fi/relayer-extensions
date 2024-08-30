//! Route handlers for the funds manager

use crate::custody_client::DepositWithdrawSource;
use crate::error::ApiError;
use crate::Server;
use bytes::Bytes;
use funds_manager_api::fees::{FeeWalletsResponse, WithdrawFeeBalanceRequest};
use funds_manager_api::gas::{
    CreateGasWalletResponse, RefillGasRequest, RegisterGasWalletRequest, RegisterGasWalletResponse,
    ReportActivePeersRequest, WithdrawGasRequest,
};
use funds_manager_api::hot_wallets::{
    CreateHotWalletRequest, CreateHotWalletResponse, HotWalletBalancesResponse,
    TransferToVaultRequest, WithdrawToHotWalletRequest,
};
use funds_manager_api::quoters::{
    DepositAddressResponse, ExecuteSwapRequest, ExecuteSwapResponse, GetExecutionQuoteRequest,
    GetExecutionQuoteResponse, WithdrawFundsRequest,
};
use itertools::Itertools;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;
use warp::reply::Json;

/// The "mints" query param
pub const MINTS_QUERY_PARAM: &str = "mints";
/// The asset used for gas (ETH)
pub const GAS_ASSET_NAME: &str = "ETH";
/// The maximum amount of gas that can be withdrawn at a given time
pub const MAX_GAS_WITHDRAWAL_AMOUNT: f64 = 1.; // ETH
/// The maximum amount that a request may refill gas to
pub const MAX_GAS_REFILL_AMOUNT: f64 = 0.1; // ETH
/// The maximum value of a quoter withdrawal that can be processed in a single
/// request
pub const MAX_WITHDRAWAL_VALUE: f64 = 50_000.; // USD

// --- Fee Indexing --- //

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

// --- Quoters --- //

/// Handler for withdrawing funds from custody
pub(crate) async fn quoter_withdraw_handler(
    withdraw_request: WithdrawFundsRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // Get the price of the token
    let maybe_price = server
        .relayer_client
        .get_binance_price(&withdraw_request.mint)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    if let Some(price) = maybe_price {
        // If a price was found, check that the withdrawal value is less than the
        // allowable maximum. If no price was found, we do not block the
        // withdrawal.

        let value = withdraw_request.amount * price;
        if value > MAX_WITHDRAWAL_VALUE {
            return Err(warp::reject::custom(ApiError::BadRequest(format!(
                "Requested withdrawal of ${} of {} exceeds maximum allowed withdrawal of ${}",
                value, withdraw_request.mint, MAX_WITHDRAWAL_VALUE
            ))));
        }
    } else {
        warn!("No price found for {}, allowing withdrawal", withdraw_request.mint);
    }

    server
        .custody_client
        .withdraw_from_hot_wallet(
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
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let address = server
        .custody_client
        .get_deposit_address(DepositWithdrawSource::Quoter)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for getting an execution quote
pub(crate) async fn get_execution_quote_handler(
    req: GetExecutionQuoteRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // Fetch the quoter hot wallet information
    let vault = DepositWithdrawSource::Quoter.vault_name();
    let hot_wallet = server.custody_client.get_hot_wallet_by_vault(vault).await?;
    let wallet = server.custody_client.get_hot_wallet_private_key(&hot_wallet.address).await?;
    let quote = server
        .execution_client
        .get_quote(req.buy_token_address, req.sell_token_address, req.sell_amount, &wallet)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = GetExecutionQuoteResponse { quote };
    Ok(warp::reply::json(&resp))
}

/// Handler for executing a swap
pub(crate) async fn execute_swap_handler(
    req: ExecuteSwapRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let vault = DepositWithdrawSource::Quoter.vault_name();
    let hot_wallet = server.custody_client.get_hot_wallet_by_vault(vault).await?;
    let wallet = server.custody_client.get_hot_wallet_private_key(&hot_wallet.address).await?;

    let tx = server.execution_client.execute_swap(req.quote, &wallet).await?;
    let resp = ExecuteSwapResponse { tx_hash: format!("{tx:#x}") };
    Ok(warp::reply::json(&resp))
}

// --- Gas --- //

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
        .withdraw_gas(withdraw_request.amount, &withdraw_request.destination_address)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    Ok(warp::reply::json(&"Withdrawal complete"))
}

/// Handler for refilling gas for all active wallets
pub(crate) async fn refill_gas_handler(
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

    server.custody_client.refill_gas_wallets(req.amount).await?;
    let resp = json!({});
    Ok(warp::reply::json(&resp))
}

/// Handler for creating a new gas wallet
pub(crate) async fn create_gas_wallet_handler(
    _body: Bytes, // no body
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let address = server
        .custody_client
        .create_gas_wallet()
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    let resp = CreateGasWalletResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for registering a gas wallet for a peer
pub(crate) async fn register_gas_wallet_handler(
    req: RegisterGasWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let key = server
        .custody_client
        .register_gas_wallet(&req.peer_id)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    let resp = RegisterGasWalletResponse { key };
    Ok(warp::reply::json(&resp))
}

/// Handler for reporting active peers
pub(crate) async fn report_active_peers_handler(
    req: ReportActivePeersRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    server
        .custody_client
        .record_active_gas_wallet(req.peers)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = json!({});
    Ok(warp::reply::json(&resp))
}

// --- Hot Wallets --- //

/// Handler for creating a hot wallet
pub(crate) async fn create_hot_wallet_handler(
    req: CreateHotWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let address = server
        .custody_client
        .create_hot_wallet(req.vault, req.internal_wallet_id)
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

/// Handler for transferring funds from a hot wallet to its backing vault
pub(crate) async fn transfer_to_vault_handler(
    req: TransferToVaultRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    server
        .custody_client
        .transfer_from_hot_wallet_to_vault(&req.hot_wallet_address, &req.mint, req.amount)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&"Transfer from hot wallet to vault initiated"))
}

/// Handler for withdrawing funds from a vault to its hot wallet
pub(crate) async fn withdraw_from_vault_handler(
    req: WithdrawToHotWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    server
        .custody_client
        .transfer_from_vault_to_hot_wallet(&req.vault, &req.mint, req.amount)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    Ok(warp::reply::json(&"Withdrawal from vault to hot wallet initiated"))
}
