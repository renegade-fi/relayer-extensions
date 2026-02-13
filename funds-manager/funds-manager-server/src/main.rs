//! The fee sweeper, sweeps for unredeemed fees in the Renegade protocol and
//! redeems them
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]
#![feature(trait_alias)]

pub mod cli;
pub mod custody_client;
pub mod db;
pub mod error;
pub mod execution_client;
// pub mod fee_indexer;
pub mod handlers;
pub mod helpers;
pub mod metrics;
pub mod middleware;
pub mod relayer_client;
pub mod server;

use clap::Parser;
use cli::Cli;
use custody_client::rpc_shim::JsonRpcRequest;
// use fee_indexer::Indexer;
use funds_manager_api::PING_ROUTE;
use funds_manager_api::fees::{
    GET_FEE_HOT_WALLET_ADDRESS_ROUTE, GET_FEE_WALLETS_ROUTE, GET_UNREDEEMED_FEE_TOTALS_ROUTE,
    INDEX_FEES_ROUTE, REDEEM_FEES_ROUTE, WITHDRAW_FEE_BALANCE_ROUTE, WithdrawFeeBalanceRequest,
};
use funds_manager_api::gas::{
    GET_GAS_HOT_WALLET_ADDRESS_ROUTE, REFILL_GAS_ROUTE, REFILL_GAS_SPONSOR_ROUTE,
    REGISTER_GAS_WALLET_ROUTE, REPORT_ACTIVE_PEERS_ROUTE, RefillGasRequest,
    RegisterGasWalletRequest, ReportActivePeersRequest, SET_GAS_WALLET_STATUS_ROUTE,
    SetGasWalletStatusRequest, WITHDRAW_GAS_ROUTE, WithdrawGasRequest,
};
use funds_manager_api::hot_wallets::{
    CreateHotWalletRequest, TRANSFER_TO_VAULT_ROUTE, TransferToVaultRequest,
    WITHDRAW_TO_HOT_WALLET_ROUTE, WithdrawToHotWalletRequest,
};
use funds_manager_api::quoters::{
    GET_DEPOSIT_ADDRESS_ROUTE, QuoteParams, SWAP_IMMEDIATE_ROUTE, SWAP_INTO_TARGET_TOKEN_ROUTE,
    SwapIntoTargetTokenRequest, WITHDRAW_CUSTODY_ROUTE, WITHDRAW_TO_HYPERLIQUID_ROUTE,
    WithdrawFundsRequest, WithdrawToHyperliquidRequest,
};
use funds_manager_api::vaults::{GET_VAULT_BALANCES_ROUTE, GetVaultBalancesRequest};
use middleware::{identity, with_chain_and_json_body, with_hmac_auth, with_json_body};
use renegade_types_core::Chain;
use server::Server;

use std::{collections::HashMap, error::Error, sync::Arc};
use tracing::{error, info_span, warn};
use warp::Filter;

use crate::custody_client::CustodyClient;
use crate::error::ApiError;
use crate::handlers::fee_indexing::{
    get_fee_hot_wallet_address_handler, get_fee_wallets_handler, get_unredeemed_fee_totals_handler,
    index_fees_handler, redeem_fees_handler, withdraw_fee_balance_handler,
};
use crate::handlers::gas::{
    create_gas_wallet_handler, get_gas_hot_wallet_address_handler, get_gas_wallets_handler,
    refill_gas_handler, refill_gas_sponsor_handler, register_gas_wallet_handler,
    report_active_peers_handler, set_gas_wallet_status_handler, withdraw_gas_handler,
};
use crate::handlers::hot_wallets::{create_hot_wallet_handler, get_hot_wallet_balances_handler};
use crate::handlers::quoters::{
    get_deposit_address_handler, quoter_withdraw_handler, withdraw_to_hyperliquid_handler,
};
use crate::handlers::rpc::rpc_handler;
use crate::handlers::swap::{swap_immediate_handler, swap_into_target_token_handler};
use crate::handlers::vaults::{
    get_vault_balances_handler, transfer_to_vault_handler, withdraw_from_vault_handler,
};

// -------
// | Cli |
// -------

/// The runtime stack size to use for the server
const RUNTIME_STACK_SIZE: usize = 50 * 1024 * 1024; // 50MB

fn main() -> Result<(), Box<dyn Error>> {
    // Create a custom tokio runtime with 50MB stack size.
    // We sometimes see stack overflows in debug mode; so we manually setup the
    // stack
    tokio::runtime::Builder::new_multi_thread()
        .thread_stack_size(RUNTIME_STACK_SIZE)
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime")
        .block_on(async_main())
}

/// Async main function
async fn async_main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    cli.validate()?;
    if cli.hmac_key.is_none() {
        warn!("Authentication is disabled. This is not recommended for production use.");
    }

    cli.configure_telemetry()?;

    let chain_env = cli.environment.to_string();
    let _root_span = info_span!("funds-manager", chain_env = %chain_env).entered();

    let port = cli.port; // copy `cli.port` to use after moving `cli`
    let server = Server::build_from_cli(cli).await.expect("failed to build server");

    // ----------
    // | Routes |
    // ----------

    let server = Arc::new(server);
    let ping = warp::get()
        .and(warp::path(PING_ROUTE))
        .map(|| warp::reply::with_status("PONG", warp::http::StatusCode::OK));

    // --- Fee Indexing --- //

    let index_fees = warp::post()
        .and(warp::path("fees"))
        .and(with_chain_param())
        .and(warp::path(INDEX_FEES_ROUTE))
        .and(with_server(server.clone()))
        .and_then(index_fees_handler);

    let redeem_fees = warp::post()
        .and(warp::path("fees"))
        .and(with_chain_param())
        .and(warp::path(REDEEM_FEES_ROUTE))
        .and(with_server(server.clone()))
        .and_then(redeem_fees_handler);

    let get_balances = warp::get()
        .and(warp::path("fees"))
        .and(with_chain_param())
        .and(warp::path(GET_FEE_WALLETS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(get_fee_wallets_handler);

    let withdraw_fee_balance = warp::post()
        .and(warp::path("fees"))
        .and(with_chain_param())
        .and(warp::path(WITHDRAW_FEE_BALANCE_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<WithdrawFeeBalanceRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(withdraw_fee_balance_handler);

    let get_fee_hot_wallet_address = warp::get()
        .and(warp::path("fees"))
        .and(with_chain_param())
        .and(warp::path(GET_FEE_HOT_WALLET_ADDRESS_ROUTE))
        .and(with_server(server.clone()))
        .and_then(get_fee_hot_wallet_address_handler);

    let get_unredeemed_fee_totals = warp::get()
        .and(warp::path("fees"))
        .and(with_chain_param())
        .and(warp::path(GET_UNREDEEMED_FEE_TOTALS_ROUTE))
        .and(with_server(server.clone()))
        .and_then(get_unredeemed_fee_totals_handler);

    // --- Vaults --- //

    let get_vault_balances = warp::post()
        .and(warp::path("custody"))
        .and(warp::path(GET_VAULT_BALANCES_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<GetVaultBalancesRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(get_vault_balances_handler);

    // --- Quoters --- //

    let withdraw_custody = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("quoters"))
        .and(warp::path(WITHDRAW_CUSTODY_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<WithdrawFundsRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(quoter_withdraw_handler);

    let get_deposit_address = warp::get()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("quoters"))
        .and(warp::path(GET_DEPOSIT_ADDRESS_ROUTE))
        .and(with_server(server.clone()))
        .and_then(get_deposit_address_handler);

    let swap_immediate = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("quoters"))
        .and(warp::path(SWAP_IMMEDIATE_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<QuoteParams>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(swap_immediate_handler);

    let swap_into_target_token = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("quoters"))
        .and(warp::path(SWAP_INTO_TARGET_TOKEN_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<SwapIntoTargetTokenRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(swap_into_target_token_handler);

    let withdraw_to_hyperliquid = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(WITHDRAW_TO_HYPERLIQUID_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawToHyperliquidRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(withdraw_to_hyperliquid_handler);

    // --- Gas --- //

    let withdraw_gas = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas"))
        .and(warp::path(WITHDRAW_GAS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<WithdrawGasRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(withdraw_gas_handler);

    let refill_gas = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas"))
        .and(warp::path(REFILL_GAS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<RefillGasRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(refill_gas_handler);

    let set_gas_wallet_status = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas-wallets"))
        .and(warp::path(SET_GAS_WALLET_STATUS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<SetGasWalletStatusRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(set_gas_wallet_status_handler);

    let add_gas_wallet = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas-wallets"))
        .and(warp::path::end())
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(create_gas_wallet_handler);

    let register_gas_wallet = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas-wallets"))
        .and(warp::path(REGISTER_GAS_WALLET_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<RegisterGasWalletRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(register_gas_wallet_handler);

    let report_active_peers = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas-wallets"))
        .and(warp::path(REPORT_ACTIVE_PEERS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<ReportActivePeersRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(report_active_peers_handler);

    let refill_gas_sponsor = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas"))
        .and(warp::path(REFILL_GAS_SPONSOR_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(refill_gas_sponsor_handler);

    let get_gas_wallets = warp::get()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas-wallets"))
        .and(warp::path::end())
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(get_gas_wallets_handler);

    let get_gas_hot_wallet_address = warp::get()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("gas"))
        .and(warp::path(GET_GAS_HOT_WALLET_ADDRESS_ROUTE))
        .and(with_server(server.clone()))
        .and_then(get_gas_hot_wallet_address_handler);

    // --- Hot Wallets --- //

    let create_hot_wallet = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("hot-wallets"))
        .and(warp::path::end())
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<CreateHotWalletRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(create_hot_wallet_handler);

    let get_hot_wallet_balances = warp::get()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("hot-wallets"))
        .and(with_hmac_auth(server.clone()))
        .and(warp::query::<HashMap<String, String>>())
        .and(with_server(server.clone()))
        .and_then(get_hot_wallet_balances_handler);

    let transfer_to_vault = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("hot-wallets"))
        .and(warp::path(TRANSFER_TO_VAULT_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<TransferToVaultRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(transfer_to_vault_handler);

    let transfer_to_hot_wallet = warp::post()
        .and(warp::path("custody"))
        .and(with_chain_param())
        .and(warp::path("hot-wallets"))
        .and(warp::path(WITHDRAW_TO_HOT_WALLET_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_chain_and_json_body::<WithdrawToHotWalletRequest>)
        .and_then(identity)
        .untuple_one()
        .and(with_server(server.clone()))
        .and_then(withdraw_from_vault_handler);

    // --- RPC --- //
    let rpc = warp::post()
        .and(warp::path("rpc"))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<JsonRpcRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(rpc_handler);

    let routes = ping
        .or(index_fees)
        .or(redeem_fees)
        .or(get_vault_balances)
        .or(withdraw_custody)
        .or(get_deposit_address)
        .or(swap_immediate)
        .or(swap_into_target_token)
        .or(withdraw_to_hyperliquid)
        .or(withdraw_gas)
        .or(refill_gas)
        .or(report_active_peers)
        .or(refill_gas_sponsor)
        .or(register_gas_wallet)
        .or(set_gas_wallet_status)
        .or(add_gas_wallet)
        .or(get_gas_wallets)
        .or(get_gas_hot_wallet_address)
        .or(get_balances)
        .or(withdraw_fee_balance)
        .or(get_fee_hot_wallet_address)
        .or(get_unredeemed_fee_totals)
        .or(transfer_to_vault)
        .or(transfer_to_hot_wallet)
        .or(get_hot_wallet_balances)
        .or(create_hot_wallet)
        .or(rpc)
        .boxed()
        .with(warp::trace(|info: warp::trace::Info| {
            tracing::info_span!(
                "request",
                method = %info.method(),
                path = %info.path(),
                chain = tracing::field::Empty,
            )
        }))
        .recover(handle_rejection);

    warp::serve(routes).run(([0, 0, 0, 0], port)).await;

    Ok(())
}

// -----------
// | Helpers |
// -----------

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: warp::Rejection) -> Result<impl warp::Reply, warp::Rejection> {
    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message) = match api_error {
            ApiError::IndexingError(msg) => (warp::http::StatusCode::BAD_REQUEST, msg),
            ApiError::RedemptionError(msg) => (warp::http::StatusCode::BAD_REQUEST, msg),
            ApiError::InternalError(msg) => (warp::http::StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::BadRequest(msg) => (warp::http::StatusCode::BAD_REQUEST, msg),
            ApiError::Unauthenticated(msg) => (warp::http::StatusCode::UNAUTHORIZED, msg),
        };
        error!("API Error: {:?}", api_error);
        Ok(warp::reply::with_status(message.clone(), code))
    } else {
        Err(err)
    }
}

/// Helper function to clone and pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}

/// Extract a `Chain` path parameter and record it on the current tracing span
fn with_chain_param() -> impl Filter<Extract = (Chain,), Error = warp::Rejection> + Clone {
    warp::path::param::<Chain>().map(|chain: Chain| {
        tracing::Span::current().record("chain", tracing::field::display(&chain));
        chain
    })
}
