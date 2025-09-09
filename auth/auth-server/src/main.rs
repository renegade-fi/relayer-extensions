//! The relayer authentication server
//!
//! This server is run independently of the relayer and is responsible for
//! issuing and managing API keys that provide access to the relayer's API.
//!
//! As such, the server holds the relayer admin key, and proxies authenticated
//! requests to the relayer directly
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::unused_async)]
#![feature(trivial_bounds)]
#![feature(let_chains)]
#![feature(duration_constructors)]
#![feature(int_roundings)]

mod bundle_store;
mod chain_events;
pub(crate) mod error;
pub mod http_utils;
mod server;
mod telemetry;

use renegade_common::types::chain::Chain;
use renegade_system_clock::SystemClock;

use auth_server_api::API_KEYS_PATH;
use clap::Parser;
use reqwest::StatusCode;
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tracing::{error, info, info_span};
use uuid::Uuid;
use warp::{
    Filter, Rejection,
    reply::{Json, WithStatus},
};

use server::Server;

use crate::error::AuthServerError;

/// The default internal server error message
const DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal Server Error";

// -------
// | CLI |
// -------

/// The command line arguments for the auth server
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    // -----------------------
    // | Environment Configs |
    // -----------------------
    /// The database url
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,
    /// The encryption key used to encrypt/decrypt database values
    #[arg(long, env = "ENCRYPTION_KEY")]
    pub encryption_key: String,
    /// The management key for the auth server, used to authenticate management
    /// requests
    #[arg(long, env = "MANAGEMENT_KEY")]
    pub management_key: String,
    /// The URL of the relayer
    #[arg(long, env = "RELAYER_URL")]
    pub relayer_url: String,
    /// The admin key for the relayer
    #[arg(long, env = "RELAYER_ADMIN_KEY")]
    pub relayer_admin_key: String,
    /// The port to run the server on
    #[arg(long, env = "PORT", default_value = "3000")]
    pub port: u16,
    /// The chain that the relayer settles to
    #[arg(long, env = "CHAIN_ID")]
    pub chain_id: Chain,
    /// The bundle rate limit in bundles per minute
    #[arg(long, env = "BUNDLE_RATE_LIMIT", default_value = "200")]
    pub bundle_rate_limit: u64,
    /// The quote rate limit in quotes per minute
    #[arg(long, env = "QUOTE_RATE_LIMIT", default_value = "500")]
    pub quote_rate_limit: u64,
    /// The path to the file containing token remaps for the given chain
    ///
    /// See https://github.com/renegade-fi/token-mappings for more information on the format of this file
    #[arg(long, env = "TOKEN_REMAP_FILE")]
    pub token_remap_file: Option<String>,
    /// The Ethereum RPC node websocket address to dial for on-chain data
    #[clap(long = "eth-websocket-url", value_parser, env = "ETH_WEBSOCKET_URL")]
    pub eth_websocket_addr: Option<String>,
    /// The RPC url to use
    #[clap(short, long, env = "RPC_URL")]
    rpc_url: String,
    /// The address of the darkpool contract
    #[clap(short, long, env = "DARKPOOL_ADDRESS")]
    darkpool_address: String,
    /// The URL of the price reporter
    #[arg(long, env = "PRICE_REPORTER_URL")]
    pub price_reporter_url: String,
    /// The URL of the Redis cluster
    #[arg(long, env = "REDIS_URL", default_value = "redis://localhost:6379")]
    pub redis_url: String,
    /// The URL of the execution cost Redis cluster
    #[arg(long, env = "EXECUTION_COST_REDIS_URL", default_value = "redis://localhost:6379")]
    pub execution_cost_redis_url: String,

    // -------------------
    // | Gas Sponsorship |
    // -------------------
    /// The address of the gas sponsor contract
    #[clap(long, env = "GAS_SPONSOR_ADDRESS")]
    gas_sponsor_address: String,
    /// The auth private key used for gas sponsorship, encoded as a hex string
    #[clap(long, env = "GAS_SPONSOR_AUTH_KEY")]
    gas_sponsor_auth_key: String,
    /// The maximum dollar value of gas sponsorship funds per day
    #[arg(long, env = "MAX_GAS_SPONSORSHIP_VALUE", default_value = "25.0")]
    max_gas_sponsorship_value: f64,
    /// The minimum quote amount for which gas sponsorship is allowed, in USD
    #[arg(long, env = "MIN_SPONSORED_ORDER_QUOTE_AMOUNT", default_value = "10.0")]
    min_sponsored_order_quote_amount: f64,

    // -------------
    // | Telemetry |
    // -------------
    /// Whether or not to enable Datadog-formatted logs
    #[arg(long, env = "ENABLE_DATADOG")]
    pub datadog_enabled: bool,
    /// Whether or not to enable OTLP tracing
    #[arg(long, env = "ENABLE_OTLP")]
    pub otlp_enabled: bool,
    /// Whether or not to enable metrics collection
    #[arg(long, env = "ENABLE_METRICS")]
    pub metrics_enabled: bool,
    /// The StatsD recorder host to send metrics to
    #[arg(long, env = "STATSD_HOST", default_value = "127.0.0.1")]
    pub statsd_host: String,
    /// The StatsD recorder port to send metrics to
    #[arg(long, env = "STATSD_PORT", default_value = "8125")]
    pub statsd_port: u16,

    /// Rate at which to sample metrics (0.0 to 1.0)
    #[arg(long, env = "METRICS_SAMPLING_RATE")]
    pub metrics_sampling_rate: Option<f64>,
}

// -------------
// | Api Types |
// -------------

/// Custom error type for API errors
#[derive(Error, Debug)]
pub enum ApiError {
    /// An internal server error
    #[error("Internal server error: {0}")]
    InternalError(String),
    /// A bad request error
    #[error("Bad request: {0}")]
    BadRequest(String),
    /// A rate limit exceeded error
    #[error("Rate limit exceeded")]
    TooManyRequests,
    /// An unauthorized error
    #[error("Unauthorized")]
    Unauthorized,
}

impl ApiError {
    /// Create a new internal server error
    #[allow(clippy::needless_pass_by_value)]
    pub fn internal<T: ToString>(msg: T) -> Self {
        Self::InternalError(msg.to_string())
    }

    /// Create a new bad request error
    #[allow(clippy::needless_pass_by_value)]
    pub fn bad_request<T: ToString>(msg: T) -> Self {
        Self::BadRequest(msg.to_string())
    }
}

// Implement warp::reject::Reject for ApiError
impl warp::reject::Reject for ApiError {}

// ----------
// | Server |
// ----------

/// The main function for the auth server
#[tokio::main]
async fn main() {
    // Set the default crypto provider for the process, this will be used by
    // websocket listeners
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let args = Cli::parse();
    let listen_addr: SocketAddr = ([0, 0, 0, 0], args.port).into();

    let system_clock = SystemClock::new().await;

    // Create the server
    let server_inner = Server::setup(args, &system_clock).await.expect("Failed to create server");
    let server = Arc::new(server_inner);

    // --- Management Routes --- //

    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // Get all API keys
    let get_all_keys = warp::path(API_KEYS_PATH)
        .and(warp::path::end())
        .and(warp::get())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(with_server(server.clone()))
        .and_then(|path, headers, server: Arc<Server>| async move {
            server.get_all_keys(path, headers).await
        });

    // Add an API key
    let add_api_key = warp::path(API_KEYS_PATH)
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.add_key(path, headers, body).await
        });

    // Expire an API key
    let expire_api_key = warp::path(API_KEYS_PATH)
        .and(warp::path::param::<Uuid>())
        .and(warp::path("deactivate"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|id, path, headers, body, server: Arc<Server>| async move {
            server.expire_key(id, path, headers, body).await
        });

    // Whitelist an API key
    let whitelist_api_key = warp::path(API_KEYS_PATH)
        .and(warp::path::param::<Uuid>())
        .and(warp::path("whitelist"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|id, path, headers, body, server: Arc<Server>| async move {
            server.whitelist_api_key(id, path, headers, body).await
        });

    // Remove a whitelist entry for an API key
    let remove_whitelist_entry = warp::path(API_KEYS_PATH)
        .and(warp::path::param::<Uuid>())
        .and(warp::path("remove-whitelist"))
        .and(warp::path::end())
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|id, path, headers, body, server: Arc<Server>| async move {
            server.remove_whitelist_entry(id, path, headers, body).await
        });

    // Get all user fees
    let get_all_user_fees = warp::path!("v0" / "fees" / "get-per-user-fees")
        .and(warp::get())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(with_server(server.clone()))
        .and_then(|path, headers, server: Arc<Server>| async move {
            server.get_all_user_fees(path, headers).await
        });

    // Set the default external match fee for an asset
    let set_asset_default_fee = warp::path!("v0" / "fees" / "set-asset-default-fee")
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.set_asset_default_fee(path, headers, body).await
        });

    // Set the per-user fee override for an asset
    let set_user_fee_override = warp::path!("v0" / "fees" / "set-user-fee-override")
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.set_user_fee_override(path, headers, body).await
        });

    // Remove the default external match fee for an asset
    let remove_asset_default_fee = warp::path!("v0" / "fees" / "remove-asset-default-fee")
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.remove_asset_default_fee(path, headers, body).await
        });

    // Remove the per-user fee override for an asset
    let remove_user_fee_override = warp::path!("v0" / "fees" / "remove-user-fee-override")
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.remove_user_fee_override(path, headers, body).await
        });

    // --- Proxied Routes --- //

    let external_quote_path = warp::path("v0")
        .and(warp::path("matching-engine"))
        .and(warp::path("quote"))
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_query_string())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
            server.handle_quote_request(path, headers, body, query_str).await
        });

    let external_quote_assembly_path = warp::path("v0")
        .and(warp::path("matching-engine"))
        .and(warp::path("assemble-external-match"))
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_query_string())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
            server.handle_assemble_quote_request(path, headers, body, query_str).await
        });

    let external_malleable_assembly_path = warp::path("v0")
        .and(warp::path("matching-engine"))
        .and(warp::path("assemble-malleable-external-match"))
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_query_string())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
            server.handle_assemble_malleable_quote_request(path, headers, body, query_str).await
        });

    let atomic_match_path = warp::path("v0")
        .and(warp::path("matching-engine"))
        .and(warp::path("request-external-match"))
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_query_string())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, query_str, server: Arc<Server>| async move {
            server.handle_external_match_request(path, headers, body, query_str).await
        });

    let order_book_depth_with_mint = warp::path("v0")
        .and(warp::path("order_book"))
        .and(warp::path("depth"))
        .and(warp::path::param::<String>())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(with_server(server.clone()))
        .and_then(|_mint, path, headers, server: Arc<Server>| async move {
            server.handle_order_book_request(path, headers).await
        });

    let order_book_depth = warp::path("v0")
        .and(warp::path("order_book"))
        .and(warp::path("depth"))
        .and(warp::path::end())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(with_server(server.clone()))
        .and_then(|path, headers, server: Arc<Server>| async move {
            server.handle_order_book_request(path, headers).await
        });

    let rfqt_levels_path = warp::path!("rfqt" / "v3" / "levels")
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(with_query_string())
        .and(with_server(server.clone()))
        .and_then(|path, headers, query_str, server: Arc<Server>| async move {
            server.handle_rfqt_levels_request(path, headers, query_str).await
        });

    let rfqt_quote_path = warp::path!("rfqt" / "v3" / "quote")
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.handle_rfqt_quote_request(path, headers, body).await
        });

    // Bind the server and listen
    info!("Starting auth server on port {}", listen_addr.port());
    let routes = ping
        .or(atomic_match_path)
        .or(external_quote_path)
        .or(external_quote_assembly_path)
        .or(external_malleable_assembly_path)
        .or(expire_api_key)
        .or(whitelist_api_key)
        .or(remove_whitelist_entry)
        .or(add_api_key)
        .or(get_all_keys)
        .or(get_all_user_fees)
        .or(set_asset_default_fee)
        .or(set_user_fee_override)
        .or(remove_asset_default_fee)
        .or(remove_user_fee_override)
        .or(order_book_depth_with_mint)
        .or(order_book_depth)
        .or(rfqt_levels_path)
        .or(rfqt_quote_path)
        .boxed()
        .with(with_tracing())
        .recover(handle_rejection);
    warp::serve(routes).bind(listen_addr).await;
}

/// Helper function to pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}

/// Helper function to parse the raw query string, returning an empty string
/// instead of rejecting in the case that no query string is present
fn with_query_string() -> impl Filter<Extract = (String,), Error = std::convert::Infallible> + Clone
{
    warp::query::raw().or_else(|_| async { Ok((String::new(),)) })
}

/// Custom tracing filter that creates spans for requests at info level
/// with the auth_server::request target to work with our RUST_LOG configuration
fn with_tracing() -> warp::trace::Trace<impl Fn(warp::trace::Info) -> tracing::Span + Clone> {
    warp::trace(|info| {
        let span = info_span!(
            target: "auth_server::request",
            "handle_request",
            method = %info.method(),
            path = %info.path(),
        );

        span
    })
}

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: Rejection) -> Result<WithStatus<Json>, Rejection> {
    let reply = if let Some(api_error) = err.find::<ApiError>() {
        api_error_to_reply(api_error)
    } else if let Some(auth_error) = err.find::<AuthServerError>().cloned() {
        let api_err = ApiError::from(auth_error);
        api_error_to_reply(&api_err)
    } else if err.is_not_found() {
        json_error("Not Found", StatusCode::NOT_FOUND)
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        json_error("Method Not Allowed", StatusCode::METHOD_NOT_ALLOWED)
    } else {
        error!("unhandled rejection: {:?}", err);
        json_error("Internal Server Error", StatusCode::INTERNAL_SERVER_ERROR)
    };

    Ok(reply)
}

/// Convert an `ApiError` into a reply
fn api_error_to_reply(api_error: &ApiError) -> WithStatus<Json> {
    let (code, message) = match api_error {
        ApiError::InternalError(e) => {
            error!("Internal server error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE)
        },
        ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.as_str()),
        ApiError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"),
        ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
    };

    json_error(message, code)
}

// -----------
// | Helpers |
// -----------

/// Return a json error from a string message
fn json_error(msg: &str, code: StatusCode) -> WithStatus<Json> {
    let json = json!({ "error": msg });
    warp::reply::with_status(warp::reply::json(&json), code)
}
