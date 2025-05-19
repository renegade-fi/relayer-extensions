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
use tracing::{error, info};
use uuid::Uuid;
use warp::{Filter, Rejection, Reply};

use server::Server;

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
    #[arg(long, env = "BUNDLE_RATE_LIMIT", default_value = "4")]
    pub bundle_rate_limit: u64,
    /// The shared bundle rate limit in bundles per minute
    #[arg(long, env = "SHARED_BUNDLE_RATE_LIMIT", default_value = "50")]
    pub shared_bundle_rate_limit: u64,
    /// The quote rate limit in quotes per minute
    #[arg(long, env = "QUOTE_RATE_LIMIT", default_value = "100")]
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
    #[arg(long, env = "REDIS_URL")]
    pub redis_url: String,

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
    /// Whether or not to enable metrics collection
    #[arg(long, env = "ENABLE_METRICS")]
    pub metrics_enabled: bool,
    /// The StatsD recorder host to send metrics to
    #[arg(long, env = "STATSD_HOST", default_value = "127.0.0.1")]
    pub statsd_host: String,
    /// The StatsD recorder port to send metrics to
    #[arg(long, env = "STATSD_PORT", default_value = "8125")]
    pub statsd_port: u16,
    /// Whether or not to enable quote comparison functionality
    #[arg(long, env = "ENABLE_QUOTE_COMPARISON", default_value = "false")]
    pub enable_quote_comparison: bool,
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
    let server_inner = Server::new(args, &system_clock).await.expect("Failed to create server");
    let server = Arc::new(server_inner);

    // --- Management Routes --- //

    // Ping route
    let ping = warp::path("ping")
        .and(warp::get())
        .map(|| warp::reply::with_status("PONG", StatusCode::OK));

    // Add an API key
    let add_api_key = warp::path(API_KEYS_PATH)
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
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(warp::post())
        .and(with_server(server.clone()))
        .and_then(|id, path, headers, body, server: Arc<Server>| async move {
            server.expire_key(id, path, headers, body).await
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
            server.handle_external_quote_request(path, headers, body, query_str).await
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
            server.handle_external_quote_assembly_request(path, headers, body, query_str).await
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
            server
                .handle_external_malleable_quote_assembly_request(path, headers, body, query_str)
                .await
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

    let order_book_depth = warp::path("v0")
        .and(warp::path("order_book"))
        .and(warp::path("depth"))
        .and(warp::path::param::<String>())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(with_server(server.clone()))
        .and_then(|mint, path, headers, server: Arc<Server>| async move {
            server.handle_order_book_depth_request(path, headers, mint).await
        });

    // Bind the server and listen
    info!("Starting auth server on port {}", listen_addr.port());
    let routes = ping
        .or(atomic_match_path)
        .or(external_quote_path)
        .or(external_quote_assembly_path)
        .or(external_malleable_assembly_path)
        .or(expire_api_key)
        .or(add_api_key)
        .or(order_book_depth)
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

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: Rejection) -> Result<impl Reply, Rejection> {
    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message) = match api_error {
            ApiError::InternalError(e) => {
                error!("Internal server error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE)
            },
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.as_str()),
            ApiError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded"),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized"),
        };

        Ok(json_error(message, code))
    } else if err.is_not_found() {
        Ok(json_error("Not Found", StatusCode::NOT_FOUND))
    } else {
        error!("unhandled rejection: {:?}", err);
        Ok(json_error("Internal Server Error", StatusCode::INTERNAL_SERVER_ERROR))
    }
}

// -----------
// | Helpers |
// -----------

/// Return a json error from a string message
fn json_error(msg: &str, code: StatusCode) -> impl Reply {
    let json = json!({ "error": msg });
    warp::reply::with_status(warp::reply::json(&json), code)
}
