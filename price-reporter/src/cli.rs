//! Defines the command-line arguments & parsing helpers for the price reporter

use clap::Parser;
use renegade_common::types::{chain::Chain, exchange::Exchange, hmac::HmacKey};
use renegade_util::telemetry::{configure_telemetry_with_metrics_config, metrics::MetricsConfig};

use crate::{
    errors::ServerError, exchanges::ExchangeConnectionsConfig, utils::PriceReporterConfig,
};

/// The prefix to apply to all metrics emitted by the price reporter
const METRICS_PREFIX: &str = "price-reporter";

/// The default statsd host to use for metrics
const DEFAULT_STATSD_HOST: &str = "127.0.0.1";
/// The default statsd port to use for metrics
const DEFAULT_STATSD_PORT: u16 = 8125;

/// The CLI for the price reporter
#[derive(Parser)]
pub struct Cli {
    // --- Server --- //
    /// The HTTP port
    #[clap(long, default_value = "3000", env = "HTTP_PORT")]
    pub http_port: u16,
    /// The websocket port
    #[clap(long, default_value = "4000", env = "WS_PORT")]
    pub ws_port: u16,
    /// The admin key, as a base64-encoded string.
    ///
    /// If not provided, the admin API will be disabled.
    #[clap(long, env = "ADMIN_KEY")]
    pub admin_key: Option<String>,

    // --- Environment --- //
    /// The path to the token remap file.
    ///
    /// If not provided, the remap will be fetched from Github.
    #[clap(long, env = "TOKEN_REMAP_PATH")]
    pub token_remap_path: Option<String>,
    /// The chains to use for token remappings, as a comma-separated list.
    #[clap(long, env = "CHAIN_ID", default_value = "devnet", value_delimiter = ',', num_args = 1..)]
    pub chains: Vec<Chain>,

    // --- Exchange Connections --- //
    /// The Coinbase API key
    #[clap(long, env = "CB_API_KEY")]
    pub coinbase_api_key: Option<String>,
    /// The Coinbase API secret
    #[clap(long, env = "CB_API_SECRET")]
    pub coinbase_api_secret: Option<String>,
    /// The Ethereum RPC node websocket address
    #[clap(long, env = "ETH_WS_ADDR")]
    pub eth_ws_addr: Option<String>,
    /// The exchanges to disable price reporting for, as a comma-separated list.
    #[clap(long, env = "DISABLED_EXCHANGES", default_value = "uniswapv3", value_delimiter = ',', num_args = 1..)]
    pub disabled_exchanges: Vec<Exchange>,

    // --- Telemetry --- //
    /// Whether or not to enable Datadog-formatted logs
    #[clap(long, env = "ENABLE_DATADOG")]
    pub datadog_enabled: bool,
    /// Whether or not to enable metrics collection
    #[clap(long, env = "ENABLE_METRICS")]
    pub metrics_enabled: bool,
}

impl Cli {
    /// Parse the CLI arguments into a `PriceReporterConfig`
    pub fn parse_price_reporter_config(&self) -> Result<PriceReporterConfig, ServerError> {
        let admin_key = self
            .admin_key
            .as_ref()
            .map(|key| HmacKey::from_base64_string(key))
            .transpose()
            .map_err(|e| ServerError::Serde(e.to_string()))?;

        Ok(PriceReporterConfig {
            http_port: self.http_port,
            ws_port: self.ws_port,
            admin_key,
            token_remap_path: self.token_remap_path.clone(),
            chains: self.chains.clone(),
            exchange_conn_config: ExchangeConnectionsConfig {
                coinbase_key_name: self.coinbase_api_key.clone(),
                coinbase_key_secret: self.coinbase_api_secret.clone(),
                eth_websocket_addr: self.eth_ws_addr.clone(),
            },
            disabled_exchanges: self.disabled_exchanges.clone(),
        })
    }

    /// Configure telemetry from the CLI arguments
    pub fn configure_telemetry(&self) -> Result<(), ServerError> {
        let metrics_config =
            MetricsConfig { metrics_prefix: METRICS_PREFIX.to_string(), ..Default::default() };

        configure_telemetry_with_metrics_config(
            self.datadog_enabled,
            false, // otlp_enabled
            self.metrics_enabled,
            "".to_string(), // collector_endpoint
            DEFAULT_STATSD_HOST,
            DEFAULT_STATSD_PORT,
            Some(metrics_config),
        )
        .map_err(|e| ServerError::TelemetrySetup(e.to_string()))
    }
}
