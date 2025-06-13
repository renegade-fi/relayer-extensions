//! Route handlers for the funds manager

use crate::cli::Environment;
use crate::custody_client::rpc_shim::JsonRpcRequest;
use crate::custody_client::DepositWithdrawSource;
use crate::error::{ApiError, FundsManagerError};
use crate::execution_client::swap::LIFI_DIAMOND_ADDRESS;
use crate::Server;
use alloy_primitives::Address;
use bytes::Bytes;
use funds_manager_api::fees::{FeeWalletsResponse, WithdrawFeeBalanceRequest};
use funds_manager_api::gas::{
    CreateGasWalletResponse, GasWalletsResponse, RefillGasRequest, RegisterGasWalletRequest,
    RegisterGasWalletResponse, ReportActivePeersRequest, WithdrawGasRequest,
};
use funds_manager_api::hot_wallets::{
    CreateHotWalletRequest, CreateHotWalletResponse, HotWalletBalancesResponse,
    TransferToVaultRequest, WithdrawToHotWalletRequest,
};
use funds_manager_api::quoters::{
    DepositAddressResponse, LiFiQuoteParams, SwapImmediateResponse, WithdrawFundsRequest,
    WithdrawToHyperliquidRequest,
};
use funds_manager_api::vaults::{GetVaultBalancesRequest, VaultBalancesResponse};
use itertools::Itertools;
use renegade_common::types::chain::Chain;
use renegade_common::types::token::default_chain;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, instrument, warn};
use warp::reply::Json;

// --- Constants --- //

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
/// The minimum amount of USDC that can be deposited into Hyperliquid
pub const MIN_HYPERLIQUID_DEPOSIT_AMOUNT: f64 = 5.0; // USDC

// --- Fee Indexing --- //

/// Handler for indexing fees
pub(crate) async fn index_fees_handler(
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
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

// --- Vaults --- //

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

// --- Quoters --- //

/// Handler for withdrawing funds from custody
pub(crate) async fn quoter_withdraw_handler(
    chain: Chain,
    withdraw_request: WithdrawFundsRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // Get the price of the token
    let maybe_price = server.price_reporter.get_price(&withdraw_request.mint, chain).await;

    match maybe_price {
        Ok(price) => {
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
        },
        Err(e) => {
            warn!("Error getting price for {}, allowing withdrawal: {e}", withdraw_request.mint);
        },
    }

    // Withdraw the funds
    let custody_client = server.get_custody_client(&chain)?;

    custody_client
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
    chain: Chain,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let custody_client = server.get_custody_client(&chain)?;
    let address = custody_client
        .get_deposit_address(DepositWithdrawSource::Quoter)
        .await
        .map_err(|e| warp::reject::custom(ApiError::InternalError(e.to_string())))?;

    let resp = DepositAddressResponse { address };
    Ok(warp::reply::json(&resp))
}

/// Handler for executing an immediate swap
#[instrument(skip_all)]
pub(crate) async fn swap_immediate_handler(
    chain: Chain,
    params: LiFiQuoteParams,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    let execution_client = server.get_execution_client(&chain)?;
    let custody_client = server.get_custody_client(&chain)?;
    let metrics_recorder = server.get_metrics_recorder(&chain)?;

    // Top up the quoter hot wallet gas before swapping
    custody_client.top_up_quoter_hot_wallet_gas().await?;

    let hot_wallet = custody_client.get_quoter_hot_wallet().await?;
    let wallet = custody_client.get_hot_wallet_private_key(&hot_wallet.address).await?;

    // Approve the top-level sell amount
    let sell_token_amount = params.from_amount;
    let sell_token_address: Address =
        params.from_token.parse().map_err(FundsManagerError::parse)?;

    execution_client
        .approve_erc20_allowance(
            sell_token_address,
            LIFI_DIAMOND_ADDRESS,
            sell_token_amount,
            &wallet,
        )
        .await?;

    // Execute the swap, decaying the size of the swap each time it fails to execute
    let (augmented_quote, receipt, swap_gas_cost) =
        execution_client.swap_immediate_decaying(chain, params, wallet).await?;

    // Compute swap costs and respond
    let execution_cost =
        match metrics_recorder.record_swap_cost(&receipt, &augmented_quote, swap_gas_cost).await {
            Ok(data) => data.execution_cost_usdc,
            Err(e) => {
                warn!("Failed to record swap cost metrics: {e}");
                0.0 // Default to 0 USD
            },
        };

    Ok(warp::reply::json(&SwapImmediateResponse {
        quote: augmented_quote.quote.clone(),
        tx_hash: format!("{:#x}", receipt.transaction_hash),
        execution_cost,
    }))
}

/// Handler for withdrawing USDC to Hyperliquid
pub(crate) async fn withdraw_to_hyperliquid_handler(
    req: WithdrawToHyperliquidRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // TODO: Separate out chain-agnostic hedging client from custody client
    let chain = match server.environment {
        Environment::Mainnet => Chain::ArbitrumOne,
        Environment::Testnet => Chain::ArbitrumSepolia,
    };

    if req.amount < MIN_HYPERLIQUID_DEPOSIT_AMOUNT {
        return Err(warp::reject::custom(ApiError::BadRequest(format!(
            "Requested amount {} USDC is less than the minimum allowed deposit of {} USDC",
            req.amount, MIN_HYPERLIQUID_DEPOSIT_AMOUNT
        ))));
    }

    let custody_client = server.get_custody_client(&chain)?;
    custody_client.withdraw_to_hyperliquid(req.amount).await?;

    Ok(warp::reply::json(&"Withdrawal to Hyperliquid complete"))
}

// --- Gas --- //

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
    custody_client.refill_gas_sponsor().await?;

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

    let addresses = gas_wallets.into_iter().map(|wallet| wallet.address).collect();
    let resp = GasWalletsResponse { addresses };

    Ok(warp::reply::json(&resp))
}

// --- Hot Wallets --- //

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

// --- RPC --- //

/// Handler for the RPC shim
pub(crate) async fn rpc_handler(
    req: JsonRpcRequest,
    server: Arc<Server>,
) -> Result<Json, warp::Rejection> {
    // TODO: Have chain-agnostic hedging client subsume this
    let chain = match server.environment {
        Environment::Mainnet => Chain::ArbitrumOne,
        Environment::Testnet => Chain::ArbitrumSepolia,
    };

    let custody_client = server.get_custody_client(&chain)?;
    let rpc_response = custody_client.handle_rpc_request(req).await;

    Ok(warp::reply::json(&rpc_response))
}
