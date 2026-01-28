//! Command-line interface for the darkpool indexer

use clap::Parser;
use renegade_types_core::Chain;
use renegade_util::telemetry::{configure_telemetry_with_metrics_config, metrics::MetricsConfig};

use crate::indexer::error::IndexerError;

// -------------
// | Constants |
// -------------

/// The prefix for metrics
const METRICS_PREFIX: &str = "darkpool-indexer";

/// The statsd host to use for metrics
const DEFAULT_STATSD_HOST: &str = "127.0.0.1";
/// The statsd port to use for metrics
const DEFAULT_STATSD_PORT: u16 = 8125;
/// The default OTLP collector endpoint
const DEFAULT_OTLP_COLLECTOR_ENDPOINT: &str = "http://localhost:4317";

// ------------------
// | CLI Definition |
// ------------------

/// The darkpool indexer CLI
#[rustfmt::skip]
#[derive(Parser)]
#[clap(about = "Darkpool indexer")]
pub struct Cli {
    // ------------
    // | Database |
    // ------------

    /// The database URL
    #[clap(long, env = "DATABASE_URL")]
    pub database_url: String,

    // ---------------
    // | HTTP Server |
    // ---------------

    /// The port to run the HTTP server on
    #[clap(long, default_value = "3000")]
    pub http_port: u16,
    /// The authentication key for the HTTP server, base64-encoded.
    /// 
    /// If not provided, the HTTP server will not be authenticated.
    #[clap(long, env = "HTTP_AUTH_KEY")]
    pub auth_key: Option<String>,

    // -----------
    // | AWS SQS |
    // -----------

    /// The URL of the AWS SQS queue
    #[clap(long, env = "SQS_QUEUE_URL")]
    pub sqs_queue_url: String,
    /// The AWS region in which the SQS queue is located
    #[clap(long, env = "SQS_REGION", default_value = "us-east-2")]
    pub sqs_region: String,

    // --------------
    // | Blockchain |
    // --------------

    /// The chain for which to index darkpool state
    #[clap(long, env = "CHAIN")]
    pub chain: Chain,
    /// The Websocket RPC URL to use for listening to onchain events
    #[clap(long, env = "WS_RPC_URL")]
    pub ws_rpc_url: String,

    // -------------
    // | Telemetry |
    // -------------

    /// Whether or not to forward telemetry to Datadog
    #[clap(long, env = "ENABLE_DATADOG")]
    pub datadog_enabled: bool,
}

impl Cli {
    /// Configure the telemetry stack for the indexer
    pub fn configure_telemetry(&self) -> Result<(), IndexerError> {
        let metrics_config =
            MetricsConfig { metrics_prefix: METRICS_PREFIX.to_string(), ..Default::default() };

        configure_telemetry_with_metrics_config(
            self.datadog_enabled, // datadog_enabled
            self.datadog_enabled, // otlp_enabled
            self.datadog_enabled, // metrics_enabled
            DEFAULT_OTLP_COLLECTOR_ENDPOINT.to_string(),
            DEFAULT_STATSD_HOST,
            DEFAULT_STATSD_PORT,
            Some(metrics_config),
        )
        .map_err(IndexerError::telemetry)
    }
}
