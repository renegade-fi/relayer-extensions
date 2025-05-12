//! CLI argument definition & parsing for the funds manager server

use aws_config::SdkConfig;
use clap::Parser;
use renegade_common::types::hmac::HmacKey;
use renegade_darkpool_client::constants::Chain;
use serde::Deserialize;
use tokio::fs::read_to_string;

use crate::{error::FundsManagerError, helpers::fetch_s3_object};

// -------------
// | Constants |
// -------------

/// The name of the chain configs object in S3
const CHAIN_CONFIGS_OBJECT_NAME: &str = "chain_configs.json";

// ---------
// | Types |
// ---------

/// Chain-specific configuration
#[derive(Debug, Clone, Deserialize)]
pub struct ChainConfig {
    // --- Relayer Params --- //
    /// The URL of the relayer to use
    pub relayer_url: String,
    /// The fee decryption key to use
    pub relayer_decryption_key: String,

    // --- Darkpool Params --- //
    /// The RPC url to use
    pub rpc_url: String,
    /// The chain to which the darkpool is deployed
    pub chain: Chain,
    /// The address of the darkpool contract
    pub darkpool_address: String,
    /// The address of the gas sponsor contract
    pub gas_sponsor_address: String,
    /// The fee decryption key to use for the protocol fees
    ///
    /// This argument is not necessary, protocol fee indexing is skipped if this
    /// is omitted
    pub protocol_decryption_key: Option<String>,

    // --- Execution Venue Params --- //
    /// The execution venue api key
    pub execution_venue_api_key: String,
    /// The execution venue base url
    pub execution_venue_base_url: String,
}

/// The cli for the fee sweeper
#[rustfmt::skip]
#[derive(Parser)]
#[clap(about = "Funds manager server")]
pub struct Cli {
    // --- Authentication --- //

    /// The HMAC key to use for authentication
    #[clap(long, conflicts_with = "disable_auth", env = "HMAC_KEY")]
    pub hmac_key: Option<String>,
    /// The HMAC key to use for signing quotes
    #[clap(long, env = "QUOTE_HMAC_KEY")]
    pub quote_hmac_key: String,
    /// Whether to disable authentication
    #[clap(long, conflicts_with = "hmac_key")]
    pub disable_auth: bool,

    // --- Chain-Specific Config --- //

    /// Name of the S3 bucket from which to read chain-specific configs
    #[clap(long, env = "CHAIN_CONFIGS_BUCKET")]
    pub chain_configs_bucket: Option<String>,

    /// Path to a file containing chain-specific configs
    ///
    /// This file should be a JSON array of chain-specific configs
    #[clap(long, env = "CHAIN_CONFIGS_PATH", conflicts_with = "chain_configs_bucket")]
    pub chain_configs_path: Option<String>,

    //  --- Api Secrets --- //

    /// The database url
    #[clap(long, env = "DATABASE_URL")]
    pub db_url: String,
    /// The fireblocks api key
    #[clap(long, env = "FIREBLOCKS_API_KEY")]
    pub fireblocks_api_key: String,
    /// The fireblocks api secret
    #[clap(long, env = "FIREBLOCKS_API_SECRET")]
    pub fireblocks_api_secret: String,

    // --- Server Config --- //

    /// The port to run the server on
    #[clap(long, default_value = "3000")]
    pub port: u16,

    // --- Telemetry --- //

    /// Whether to enable datadog formatted logs
    #[clap(long, default_value = "false")]
    pub datadog_logging: bool,
    /// Whether or not to enable metrics collection
    #[clap(long, env = "ENABLE_METRICS")]
    pub metrics_enabled: bool,
    /// The StatsD recorder host to send metrics to
    #[clap(long, env = "STATSD_HOST", default_value = "127.0.0.1")]
    pub statsd_host: String,
    /// The StatsD recorder port to send metrics to
    #[clap(long, env = "STATSD_PORT", default_value = "8125")]
    pub statsd_port: u16,
}

impl Cli {
    /// Validate the CLI arguments
    pub fn validate(&self) -> Result<(), String> {
        if self.hmac_key.is_none() && !self.disable_auth {
            return Err("Either --hmac-key or --disable-auth must be provided".to_string());
        }

        if self.chain_configs_bucket.is_none() && self.chain_configs_path.is_none() {
            return Err("Either --chain-configs-bucket or --chain-configs-path must be provided"
                .to_string());
        }

        Ok(())
    }

    /// Get the HMAC key
    pub fn get_hmac_key(&self) -> Option<HmacKey> {
        self.hmac_key.as_ref().map(|key| HmacKey::from_hex_string(key).expect("Invalid HMAC key"))
    }

    /// Get the quote HMAC key
    pub fn get_quote_hmac_key(&self) -> HmacKey {
        HmacKey::from_hex_string(&self.quote_hmac_key).expect("Invalid quote HMAC key")
    }

    /// Parse the chain configs
    pub async fn parse_chain_configs(
        &self,
        aws_config: &SdkConfig,
    ) -> Result<Vec<ChainConfig>, FundsManagerError> {
        let json_str = if let Some(bucket) = &self.chain_configs_bucket {
            fetch_s3_object(bucket, CHAIN_CONFIGS_OBJECT_NAME, aws_config).await
        } else {
            read_to_string(self.chain_configs_path.as_ref().expect("no chain configs file path"))
                .await
                .map_err(FundsManagerError::custom)
        }?;

        serde_json::from_str(&json_str).map_err(FundsManagerError::parse)
    }
}
