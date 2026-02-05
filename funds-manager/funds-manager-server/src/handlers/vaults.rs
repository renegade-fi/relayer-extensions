//! Handlers for vault endpoints

use std::sync::Arc;

use funds_manager_api::{
    hot_wallets::{TransferToVaultRequest, WithdrawToHotWalletRequest},
    vaults::{GetVaultBalancesRequest, VaultBalancesResponse},
};
use renegade_types_core::{default_chain, Chain};
use warp::reply::Json;

use crate::{error::ApiError, server::Server};

/// Handler for getting the balances of a vault
pub(crate) async fn get_vault_balances_handler(
    req: GetVaultBalancesRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // The vault balances handler is chain-agnostic, so we use the default chain
    let chain = default_chain();
    let custody_client = server.get_custody_client(&chain)?;

    let balances = custody_client.get_vault_token_balances(&req.vault).await?;

    let resp = VaultBalancesResponse { balances };
    Ok(warp::reply::json(&resp))
}

/// Handler for transferring funds from a hot wallet to its backing vault
pub(crate) async fn transfer_to_vault_handler(
    chain: Chain,
    req: TransferToVaultRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;

    custody_client
        .transfer_from_hot_wallet_to_vault(&req.hot_wallet_address, &req.mint, req.amount)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    Ok(warp::reply::json(&"Transfer from hot wallet to vault initiated"))
}

/// Handler for withdrawing funds from a vault to its hot wallet
pub(crate) async fn withdraw_from_vault_handler(
    chain: Chain,
    req: WithdrawToHotWalletRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;

    custody_client
        .transfer_from_vault_to_hot_wallet(&req.vault, &req.mint, req.amount)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;
    Ok(warp::reply::json(&"Withdrawal from vault to hot wallet initiated"))
}
