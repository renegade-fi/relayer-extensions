//! The fee sweeper, sweeps for unredeemed fees in the Renegade protocol and
//! redeems them
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

pub mod custody_client;
pub mod db;
pub mod error;
pub mod execution_client;
pub mod fee_indexer;
pub mod handlers;
pub mod helpers;
pub mod metrics;
pub mod middleware;
pub mod relayer_client;
pub mod server;

use custody_client::rpc_shim::JsonRpcRequest;
use fee_indexer::Indexer;
use funds_manager_api::fees::{
    WithdrawFeeBalanceRequest, GET_FEE_WALLETS_ROUTE, INDEX_FEES_ROUTE, REDEEM_FEES_ROUTE,
    WITHDRAW_FEE_BALANCE_ROUTE,
};
use funds_manager_api::gas::{
    RefillGasRequest, RegisterGasWalletRequest, ReportActivePeersRequest, WithdrawGasRequest,
    REFILL_GAS_ROUTE, REFILL_GAS_SPONSOR_ROUTE, REGISTER_GAS_WALLET_ROUTE,
    REPORT_ACTIVE_PEERS_ROUTE, WITHDRAW_GAS_ROUTE,
};
use funds_manager_api::hot_wallets::{
    CreateHotWalletRequest, TransferToVaultRequest, WithdrawToHotWalletRequest,
    TRANSFER_TO_VAULT_ROUTE, WITHDRAW_TO_HOT_WALLET_ROUTE,
};
use funds_manager_api::quoters::{
    ExecuteSwapRequest, WithdrawFundsRequest, WithdrawToHyperliquidRequest, EXECUTE_SWAP_ROUTE,
    GET_DEPOSIT_ADDRESS_ROUTE, GET_EXECUTION_QUOTE_ROUTE, WITHDRAW_CUSTODY_ROUTE,
    WITHDRAW_TO_HYPERLIQUID_ROUTE,
};
use funds_manager_api::PING_ROUTE;
use handlers::{
    create_gas_wallet_handler, create_hot_wallet_handler, execute_swap_handler,
    get_deposit_address_handler, get_execution_quote_handler, get_fee_wallets_handler,
    get_hot_wallet_balances_handler, index_fees_handler, quoter_withdraw_handler,
    redeem_fees_handler, refill_gas_handler, refill_gas_sponsor_handler,
    register_gas_wallet_handler, report_active_peers_handler, rpc_handler,
    transfer_to_vault_handler, withdraw_fee_balance_handler, withdraw_from_vault_handler,
    withdraw_gas_handler, withdraw_to_hyperliquid_handler,
};
use middleware::{identity, with_hmac_auth, with_json_body};
use renegade_common::types::hmac::HmacKey;
use renegade_util::telemetry::configure_telemetry;
use server::Server;
use warp::Filter;

use std::{collections::HashMap, error::Error, sync::Arc};

use clap::Parser;
use renegade_arbitrum_client::constants::Chain;
use tracing::{error, warn};

use crate::custody_client::CustodyClient;
use crate::error::ApiError;

// -------
// | Cli |
// -------

/// The cli for the fee sweeper
#[rustfmt::skip]
#[derive(Parser)]
#[clap(about = "Funds manager server")]
struct Cli {
    // --- Authentication --- //

    /// The HMAC key to use for authentication
    #[clap(long, conflicts_with = "disable_auth", env = "HMAC_KEY")]
    hmac_key: Option<String>,
    /// The HMAC key to use for signing quotes
    #[clap(long, env = "QUOTE_HMAC_KEY")]
    quote_hmac_key: String,
    /// Whether to disable authentication
    #[clap(long, conflicts_with = "hmac_key")]
    disable_auth: bool,

    // --- Environment Configs --- //

    /// The URL of the relayer to use
    #[clap(long, env = "RELAYER_URL")]
    relayer_url: String,
    /// The address of the darkpool contract
    #[clap(short = 'a', long, env = "DARKPOOL_ADDRESS")]
    darkpool_address: String,
    /// The chain to redeem fees for
    #[clap(long, default_value = "mainnet", env = "CHAIN")]
    chain: Chain,
    /// The token address of the USDC token, used to get prices for fee
    /// redemption
    #[clap(long, env = "USDC_MINT")]
    usdc_mint: String,
    /// The address of the gas sponsor contract
    #[clap(long, env = "GAS_SPONSOR_ADDRESS")]
    gas_sponsor_address: String,

    // --- Decryption Keys --- //

    /// The fee decryption key to use
    #[clap(long, env = "RELAYER_DECRYPTION_KEY")]
    relayer_decryption_key: String,
    /// The fee decryption key to use for the protocol fees
    ///
    /// This argument is not necessary, protocol fee indexing is skipped if this
    /// is omitted
    #[clap(long, env = "PROTOCOL_DECRYPTION_KEY")]
    protocol_decryption_key: Option<String>,

    //  --- Api Secrets --- //

    /// The Arbitrum RPC url to use
    #[clap(short, long, env = "RPC_URL")]
    rpc_url: String,
    /// The database url
    #[clap(long, env = "DATABASE_URL")]
    db_url: String,
    /// The fireblocks api key
    #[clap(long, env = "FIREBLOCKS_API_KEY")]
    fireblocks_api_key: String,
    /// The fireblocks api secret
    #[clap(long, env = "FIREBLOCKS_API_SECRET")]
    fireblocks_api_secret: String,
    /// The execution venue api key
    #[clap(long, env = "EXECUTION_VENUE_API_KEY")]
    execution_venue_api_key: String,
    /// The execution venue base url
    #[clap(long, env = "EXECUTION_VENUE_BASE_URL")]
    execution_venue_base_url: String,

    // --- Server Config --- //

    /// The port to run the server on
    #[clap(long, default_value = "3000")]
    port: u16,

    // -------------
    // | Telemetry |
    // -------------
    /// Whether to enable datadog formatted logs
    #[clap(long, default_value = "false")]
    datadog_logging: bool,
    /// Whether or not to enable metrics collection
    #[clap(long, env = "ENABLE_METRICS")]
    metrics_enabled: bool,
    /// The StatsD recorder host to send metrics to
    #[clap(long, env = "STATSD_HOST", default_value = "127.0.0.1")]
    statsd_host: String,
    /// The StatsD recorder port to send metrics to
    #[clap(long, env = "STATSD_PORT", default_value = "8125")]
    statsd_port: u16,
}

impl Cli {
    /// Validate the CLI arguments
    fn validate(&self) -> Result<(), String> {
        if self.hmac_key.is_none() && !self.disable_auth {
            Err("Either --hmac-key or --disable-auth must be provided".to_string())
        } else {
            Ok(())
        }
    }

    /// Get the HMAC key
    fn get_hmac_key(&self) -> Option<HmacKey> {
        self.hmac_key.as_ref().map(|key| HmacKey::from_hex_string(key).expect("Invalid HMAC key"))
    }

    /// Get the quote HMAC key
    fn get_quote_hmac_key(&self) -> HmacKey {
        HmacKey::from_hex_string(&self.quote_hmac_key).expect("Invalid quote HMAC key")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    cli.validate()?;
    if cli.hmac_key.is_none() {
        warn!("Authentication is disabled. This is not recommended for production use.");
    }

    configure_telemetry(
        cli.datadog_logging, // datadog_enabled
        false,               // otlp_enabled
        cli.metrics_enabled, // metrics_enabled
        "".to_string(),      // collector_endpoint
        &cli.statsd_host,    // statsd_host
        cli.statsd_port,     // statsd_port
    )
    .expect("failed to setup telemetry");

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
        .and(warp::path(INDEX_FEES_ROUTE))
        .and(with_server(server.clone()))
        .and_then(index_fees_handler);

    let redeem_fees = warp::post()
        .and(warp::path("fees"))
        .and(warp::path(REDEEM_FEES_ROUTE))
        .and(with_server(server.clone()))
        .and_then(redeem_fees_handler);

    let get_balances = warp::get()
        .and(warp::path("fees"))
        .and(warp::path(GET_FEE_WALLETS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(get_fee_wallets_handler);

    let withdraw_fee_balance = warp::post()
        .and(warp::path("fees"))
        .and(warp::path(WITHDRAW_FEE_BALANCE_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawFeeBalanceRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(withdraw_fee_balance_handler);

    // --- Quoters --- //

    let withdraw_custody = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(WITHDRAW_CUSTODY_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawFundsRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(quoter_withdraw_handler);

    let get_deposit_address = warp::get()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(GET_DEPOSIT_ADDRESS_ROUTE))
        .and(with_server(server.clone()))
        .and_then(get_deposit_address_handler);

    let get_execution_quote = warp::get()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(GET_EXECUTION_QUOTE_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .and(warp::query::<HashMap<String, String>>())
        .and(with_server(server.clone()))
        .and_then(get_execution_quote_handler);

    let execute_swap = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(EXECUTE_SWAP_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<ExecuteSwapRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(execute_swap_handler);

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
        .and(warp::path("gas"))
        .and(warp::path(WITHDRAW_GAS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawGasRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(withdraw_gas_handler);

    let refill_gas = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas"))
        .and(warp::path(REFILL_GAS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<RefillGasRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(refill_gas_handler);

    let add_gas_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas-wallets"))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(create_gas_wallet_handler);

    let register_gas_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas-wallets"))
        .and(warp::path(REGISTER_GAS_WALLET_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<RegisterGasWalletRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(register_gas_wallet_handler);

    let report_active_peers = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas-wallets"))
        .and(warp::path(REPORT_ACTIVE_PEERS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<ReportActivePeersRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(report_active_peers_handler);

    let refill_gas_sponsor = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas"))
        .and(warp::path(REFILL_GAS_SPONSOR_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(refill_gas_sponsor_handler);

    // --- Hot Wallets --- //

    let create_hot_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<CreateHotWalletRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(create_hot_wallet_handler);

    let get_hot_wallet_balances = warp::get()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(with_hmac_auth(server.clone()))
        .and(warp::query::<HashMap<String, String>>())
        .and(with_server(server.clone()))
        .and_then(get_hot_wallet_balances_handler);

    let transfer_to_vault = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(warp::path(TRANSFER_TO_VAULT_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<TransferToVaultRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(transfer_to_vault_handler);

    let transfer_to_hot_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(warp::path(WITHDRAW_TO_HOT_WALLET_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawToHotWalletRequest>)
        .and_then(identity)
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
        .or(withdraw_custody)
        .or(get_deposit_address)
        .or(get_execution_quote)
        .or(execute_swap)
        .or(withdraw_to_hyperliquid)
        .or(withdraw_gas)
        .or(refill_gas)
        .or(report_active_peers)
        .or(refill_gas_sponsor)
        .or(register_gas_wallet)
        .or(add_gas_wallet)
        .or(get_balances)
        .or(withdraw_fee_balance)
        .or(transfer_to_vault)
        .or(transfer_to_hot_wallet)
        .or(get_hot_wallet_balances)
        .or(create_hot_wallet)
        .or(rpc)
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
        error!("Unhandled rejection: {:?}", err);
        Err(err)
    }
}

/// Helper function to clone and pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}
