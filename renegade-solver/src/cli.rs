//! The CLI for the renegade solver

use alloy_primitives::ChainId;
use clap::Parser;

use renegade_common::types::chain::Chain;
use renegade_util::telemetry::{configure_telemetry_with_metrics_config, metrics::MetricsConfig};

/// The default metrics prefix
const DEFAULT_METRICS_PREFIX: &str = "renegade-solver";
/// The default OTLP collector endpoint
const DEFAULT_OTLP_COLLECTOR_ENDPOINT: &str = "http://localhost:4317";
/// The default statsd host
const DEFAULT_STATSD_HOST: &str = "127.0.0.1";
/// The default statsd port
const DEFAULT_STATSD_PORT: u16 = 8125;

/// Renegade solver server
#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    // --- Application Config --- //
    /// The URL of the UniswapX API
    #[arg(long, env = "UNISWAPX_URL")]
    pub uniswapx_url: String,
    /// The API key for the Renegade external match API
    #[arg(long, env = "RENEGADE_API_KEY")]
    pub renegade_api_key: String,
    /// The API secret for the Renegade external match API
    #[arg(long, env = "RENEGADE_API_SECRET")]
    pub renegade_api_secret: String,
    /// The chain the solver is running on
    #[arg(long, env = "CHAIN_ID", default_value = "base-mainnet")]
    pub chain_id: Chain,

    // --- Executor Config --- //
    /// The address of the executor contract
    #[arg(long, env = "EXECUTOR_ADDRESS")]
    pub contract_address: String,
    /// The Flashblocks WebSocket URL
    #[arg(long, env = "FB_WEBSOCKET_URL")]
    pub fb_websocket_url: String,
    /// The WebSocket URL for real-time block monitoring
    #[arg(long, env = "RPC_WEBSOCKET_URL")]
    pub rpc_websocket_url: String,
    /// The private key for signing transactions
    #[arg(long, env = "PRIVATE_KEY")]
    pub private_key: String,

    // --- Server --- //
    /// Port to run the server on
    #[arg(short, long, default_value_t = 3000)]
    pub port: u16,

    // --- Telemetry --- //
    /// Whether or not to enable Datadog-formatted logs
    #[arg(long, env = "ENABLE_DATADOG")]
    pub datadog_enabled: bool,
    /// Whether or not to enable OTLP tracing
    #[arg(long, env = "ENABLE_OTLP")]
    pub otlp_enabled: bool,
    /// Whether or not to enable metrics collection
    #[arg(long, env = "ENABLE_METRICS")]
    pub metrics_enabled: bool,
}

impl Cli {
    /// Configure telemetry from the CLI
    pub fn configure_telemetry(&self) {
        let metrics_config = MetricsConfig {
            metrics_prefix: DEFAULT_METRICS_PREFIX.to_string(),
            ..Default::default()
        };

        configure_telemetry_with_metrics_config(
            self.datadog_enabled,
            self.otlp_enabled,
            self.metrics_enabled,
            DEFAULT_OTLP_COLLECTOR_ENDPOINT.to_string(),
            DEFAULT_STATSD_HOST,
            DEFAULT_STATSD_PORT,
            Some(metrics_config),
        )
        .expect("Failed to configure telemetry");
    }
}

/// Map a Chain enum to its numeric chain ID
pub fn chain_to_chain_id(c: &Chain) -> ChainId {
    match c {
        Chain::ArbitrumOne => 42161u64, // Arbitrum One Mainnet
        Chain::BaseMainnet => 8453u64,  // Base Mainnet
        _ => panic!("Unsupported chain: only ArbitrumOne and BaseMainnet are allowed"),
    }
}
