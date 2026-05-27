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
}
