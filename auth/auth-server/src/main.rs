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

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use price_reporter_client::{PriceReporterClient, PriceReporterClientConfig};
use renegade_common::types::chain::Chain;
use renegade_system_clock::SystemClock;
use thiserror::Error;
use tokio::select;
use tokio::sync::mpsc::channel;
use tracing::error;

use bundle_store::BundleStore;
use chain_events::listener::{OnChainEventListener, OnChainEventListenerConfig};
use server::gas_estimation::gas_cost_sampler::GasCostSampler;
use server::helpers::{
    create_darkpool_client, parse_gas_sponsor_address, parse_malleable_match_connector_address,
    set_external_match_fees, setup_token_mapping,
};
use server::rate_limiter::AuthServerRateLimiter;
use server::worker::{HttpServerConfig, HttpServerWorker};

use crate::error::AuthServerError;

/// The default internal server error message
const DEFAULT_INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal Server Error";

// -------
// | CLI |
// -------

/// The command line arguments for the auth server
#[derive(Parser, Debug, Clone)]
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
    /// The address of the malleable match gas sponsor connector
    #[clap(long, env = "MALLEABLE_MATCH_CONNECTOR_ADDRESS")]
    malleable_match_connector_address: String,
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
async fn main() -> Result<(), AuthServerError> {
    // Set the default crypto provider for the process, this will be used by
    // websocket listeners
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let args = Cli::parse();
    let listen_addr: SocketAddr = ([0, 0, 0, 0], args.port).into();

    let system_clock = SystemClock::new().await;

    // Setup token mappings
    setup_token_mapping(&args).await.expect("Failed to setup token mapping");

    // Create the darkpool client
    let darkpool_client =
        create_darkpool_client(args.darkpool_address.clone(), args.chain_id, args.rpc_url.clone())
            .expect("failed to create darkpool client");

    // Set the external match fees & protocol fee
    set_external_match_fees(&darkpool_client).await.expect("failed to set external match fees");

    // Parse addresses
    let gas_sponsor_address =
        parse_gas_sponsor_address(&args).expect("failed to parse gas sponsor address");
    let malleable_match_connector_address = parse_malleable_match_connector_address(&args)
        .expect("failed to parse malleable match connector address");

    // Create shared dependencies
    let bundle_store = BundleStore::new();

    let rate_limiter = AuthServerRateLimiter::new(
        args.quote_rate_limit,
        args.bundle_rate_limit,
        args.max_gas_sponsorship_value,
        &args.redis_url,
        &args.execution_cost_redis_url,
    )
    .await
    .expect("failed to create rate limiter");

    let price_reporter_client = PriceReporterClient::new(PriceReporterClientConfig {
        base_url: args.price_reporter_url.clone(),
        ..Default::default()
    })
    .expect("failed to create price reporter client");

    let gas_cost_sampler = Arc::new(
        GasCostSampler::new(darkpool_client.provider().clone(), gas_sponsor_address, &system_clock)
            .await
            .expect("failed to create gas cost sampler"),
    );

    // Start the on-chain event listener
    let chain_listener_config = OnChainEventListenerConfig {
        chain: args.chain_id,
        gas_sponsor_address,
        websocket_addr: args.eth_websocket_addr.clone(),
        bundle_store: bundle_store.clone(),
        rate_limiter: rate_limiter.clone(),
        price_reporter_client: price_reporter_client.clone(),
        gas_cost_sampler: gas_cost_sampler.clone(),
        darkpool_client: darkpool_client.clone(),
    };
    let mut chain_listener = OnChainEventListener::new(chain_listener_config)
        .expect("failed to build on-chain event listener");
    chain_listener.start().expect("failed to start on-chain event listener");
    let (chain_listener_failure_sender, mut chain_listener_failure_receiver) =
        new_worker_failure_channel();
    chain_listener.watch(&chain_listener_failure_sender);

    // Start the HTTP server worker
    let http_server_config = HttpServerConfig {
        args,
        gas_sponsor_address,
        malleable_match_connector_address,
        bundle_store: bundle_store.clone(),
        rate_limiter: rate_limiter.clone(),
        price_reporter_client: price_reporter_client.clone(),
        gas_cost_sampler: gas_cost_sampler.clone(),
        listen_addr,
    };
    let mut http_server =
        HttpServerWorker::new(http_server_config).expect("failed to build HTTP server worker");
    http_server.start().expect("failed to start HTTP server worker");
    let (http_server_failure_sender, mut http_server_failure_receiver) =
        new_worker_failure_channel();
    http_server.watch(&http_server_failure_sender);

    // Wait for an error, log the error, and teardown the auth server
    select! {
        _ = chain_listener_failure_receiver.recv() => {
            error!("Chain listener failed, shutting down");
            Err(AuthServerError::WorkerFailed("chain listener".to_string()))
        },
        _ = http_server_failure_receiver.recv() => {
            error!("HTTP server failed, shutting down");
            Err(AuthServerError::WorkerFailed("http server".to_string()))
        },
    }
}

/// Create a new worker failure channel
pub fn new_worker_failure_channel()
-> (tokio::sync::mpsc::Sender<()>, tokio::sync::mpsc::Receiver<()>) {
    channel(1 /* buffer */)
}
