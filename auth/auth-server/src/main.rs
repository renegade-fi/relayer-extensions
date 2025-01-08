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

pub(crate) mod error;
pub(crate) mod models;
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub(crate) mod schema;
mod server;
mod telemetry;

use auth_server_api::API_KEYS_PATH;
use clap::Parser;
use ethers::signers::LocalWallet;
use renegade_arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use renegade_config::setup_token_remaps;
use renegade_util::{
    err_str,
    telemetry::{configure_telemetry_with_metrics_config, metrics::MetricsConfig},
};
use reqwest::StatusCode;
use serde_json::json;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;
use tracing::{error, info};
use uuid::Uuid;
use warp::{Filter, Rejection, Reply};

use server::Server;

/// The default internal server error message
const DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal Server Error";
/// The dummy private key used to instantiate the arbitrum client
///
/// We don't need any client functionality using a real private key, so instead
/// we use the key deployed by Arbitrum on local devnets
const DUMMY_PRIVATE_KEY: &str =
    "0xb6b15c8cb491557369f3c7d2c287b053eb229daa9c22138887752191c9520659";

/// The metrics prefix for the auth server
///
/// Set to "renegade_relayer" to match existing metrics
const METRICS_PREFIX: &str = "renegade_relayer";
/// The buffer size for the metrics queue
const BUFFER_SIZE: usize = 1024;
/// The queue size for the metrics queue
const QUEUE_SIZE: usize = 1024 * 1024;

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
    /// The path to the file containing token remaps for the given chain
    ///
    /// See https://github.com/renegade-fi/token-mappings for more information on the format of this file
    #[arg(long, env = "TOKEN_REMAP_FILE")]
    pub token_remap_file: Option<String>,
    /// The Arbitrum RPC url to use
    #[clap(short, long, env = "RPC_URL")]
    rpc_url: String,
    /// The address of the darkpool contract
    #[clap(short = 'a', long, env = "DARKPOOL_ADDRESS")]
    darkpool_address: String,

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
    let args = Cli::parse();
    let listen_addr: SocketAddr = ([0, 0, 0, 0], args.port).into();

    // Configure metrics
    let metrics_config = MetricsConfig {
        metrics_prefix: METRICS_PREFIX.to_string(),
        buffer_size: BUFFER_SIZE,
        queue_size: QUEUE_SIZE,
    };

    // Setup logging
    configure_telemetry_with_metrics_config(
        args.datadog_enabled, // datadog_enabled
        false,                // otlp_enabled
        args.metrics_enabled, // metrics_enabled
        "".to_string(),       // collector_endpoint
        &args.statsd_host,    // statsd_host
        args.statsd_port,     // statsd_port
        Some(metrics_config),
    )
    .expect("failed to setup telemetry");

    // Set up the token remapping
    let chain_id = args.chain_id;
    let token_remap_file = args.token_remap_file.clone();
    tokio::task::spawn_blocking(move || {
        setup_token_remaps(token_remap_file, chain_id)
            .map_err(err_str!(error::AuthServerError::Setup))
    })
    .await
    .unwrap()
    .expect("Failed to setup token remaps");

    // Build an Arbitrum client
    let wallet =
        LocalWallet::from_str(DUMMY_PRIVATE_KEY).expect("Failed to create wallet from private key");
    let arbitrum_client = ArbitrumClient::new(ArbitrumClientConfig {
        darkpool_addr: args.darkpool_address.clone(),
        chain: args.chain_id,
        rpc_url: args.rpc_url.clone(),
        arb_priv_keys: vec![wallet],
        block_polling_interval_ms: 100,
    })
    .await
    .unwrap();

    // Create the server
    let server = Server::new(args, arbitrum_client).await.expect("Failed to create server");
    let server = Arc::new(server);

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
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.handle_external_quote_request(path, headers, body).await
        });

    let external_quote_assembly_path = warp::path("v0")
        .and(warp::path("matching-engine"))
        .and(warp::path("assemble-external-match"))
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.handle_external_quote_assembly_request(path, headers, body).await
        });

    let atomic_match_path = warp::path("v0")
        .and(warp::path("matching-engine"))
        .and(warp::path("request-external-match"))
        .and(warp::post())
        .and(warp::path::full())
        .and(warp::header::headers_cloned())
        .and(warp::body::bytes())
        .and(with_server(server.clone()))
        .and_then(|path, headers, body, server: Arc<Server>| async move {
            server.handle_external_match_request(path, headers, body).await
        });

    // Bind the server and listen
    info!("Starting auth server on port {}", listen_addr.port());
    let routes = ping
        .or(atomic_match_path)
        .or(external_quote_path)
        .or(external_quote_assembly_path)
        .or(expire_api_key)
        .or(add_api_key)
        .recover(handle_rejection);
    warp::serve(routes).bind(listen_addr).await;
}

/// Helper function to pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
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
