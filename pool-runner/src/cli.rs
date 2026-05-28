//! CLI argument parsing for the pool-runner service

use clap::Parser;

/// CLI for the pool-runner service
#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
pub struct Cli {
    /// Port to listen on for the HTTP healthcheck
    #[arg(long, env = "POOL_RUNNER_PORT", default_value = "3000")]
    pub port: u16,

    /// URL of the relayer admin API (e.g. https://relayer.example.com)
    #[arg(long, env = "RELAYER_ADMIN_URL")]
    pub relayer_admin_url: String,

    /// Base64-encoded HMAC key for relayer admin requests
    #[arg(long, env = "RELAYER_ADMIN_KEY")]
    pub relayer_admin_key: String,

    /// Chain ID of the network the relayer is running on
    #[arg(long, env = "CHAIN_ID")]
    pub chain_id: u64,

    /// Path or S3 URI (s3://bucket/key) to the runner config JSON file
    #[arg(long, env = "RUNNER_CONFIG_PATH")]
    pub runner_config_path: String,

    /// Base URL of the price reporter service (e.g. https://price-reporter.example.com)
    #[arg(long, env = "PRICE_REPORTER_URL")]
    pub price_reporter_url: String,

    // --- Telemetry --- //
    /// Emit logs as Datadog-formatted JSON (vs. pretty ANSI text). Set true
    /// in deployed environments so Datadog parses the structured fields.
    #[arg(long, env = "ENABLE_DATADOG", default_value = "false")]
    pub enable_datadog: bool,

    /// Export OTLP traces to the collector.
    #[arg(long, env = "ENABLE_OTLP", default_value = "false")]
    pub enable_otlp: bool,

    /// Record StatsD metrics.
    #[arg(long, env = "ENABLE_METRICS", default_value = "false")]
    pub enable_metrics: bool,

    /// OTLP collector endpoint (only used when `enable_otlp`).
    #[arg(long, env = "OTLP_COLLECTOR_ENDPOINT", default_value = "")]
    pub otlp_collector_endpoint: String,

    /// StatsD host (only used when `enable_metrics`).
    #[arg(long, env = "STATSD_HOST", default_value = "127.0.0.1")]
    pub statsd_host: String,

    /// StatsD port (only used when `enable_metrics`).
    #[arg(long, env = "STATSD_PORT", default_value = "8125")]
    pub statsd_port: u16,
}
