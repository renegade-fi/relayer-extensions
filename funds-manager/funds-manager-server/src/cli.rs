//! CLI argument definition & parsing for the funds manager server

use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;
use aws_config::SdkConfig;
use clap::{Parser, ValueEnum};
use price_reporter_client::PriceReporterClient;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_common::types::{chain::Chain, hmac::HmacKey};
use renegade_darkpool_client::client::DarkpoolClientConfig;
use serde::Deserialize;
use tokio::fs::read_to_string;

use crate::{
    custody_client::CustodyClient, db::DbPool, error::FundsManagerError,
    execution_client::ExecutionClient, helpers::fetch_s3_object, metrics::MetricsRecorder,
    mux_darkpool_client::MuxDarkpoolClient, relayer_client::RelayerClient, Indexer,
};

// -------------
// | Constants |
// -------------

/// The name of the chain configs object in S3
const CHAIN_CONFIGS_OBJECT_NAME: &str = "chain_configs.json";

/// The block polling interval for the darkpool client
const BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

// ---------
// | Types |
// ---------

/// A type alias for a map of chain configs
type ChainConfigsMap = HashMap<Chain, ChainConfig>;

/// The (chain-agnostic) environment the server is running in.
#[derive(Clone, ValueEnum)]
pub enum Environment {
    /// The mainnet environment
    Mainnet,
    /// The testnet environment
    Testnet,
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

    // --- Chain-Agnostic Config --- //

    /// The environment to run the server in
    #[clap(long, env = "ENVIRONMENT", default_value = "testnet")]
    pub environment: Environment,
    /// The URL of the price reporter
    #[clap(long, env = "PRICE_REPORTER_URL")]
    pub price_reporter_url: String,

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
    ) -> Result<ChainConfigsMap, FundsManagerError> {
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

/// Funds manager configuration options for a given chain
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

impl ChainConfig {
    /// Build chain-specific clients from the given config
    pub async fn build_clients(
        &self,
        chain: Chain,
        fireblocks_api_key: String,
        fireblocks_api_secret: String,
        db_pool: Arc<DbPool>,
        aws_config: SdkConfig,
        price_reporter: Arc<PriceReporterClient>,
    ) -> Result<ChainClients, FundsManagerError> {
        // Build a relayer client
        let relayer_client = RelayerClient::new(&self.relayer_url, chain);

        // Build a darkpool client
        let private_key = PrivateKeySigner::random();
        let conf = DarkpoolClientConfig {
            darkpool_addr: self.darkpool_address.clone(),
            chain,
            rpc_url: self.rpc_url.clone(),
            private_key,
            block_polling_interval: BLOCK_POLLING_INTERVAL,
        };
        let darkpool_client =
            MuxDarkpoolClient::new(chain, conf).map_err(FundsManagerError::custom)?;
        let chain_id = darkpool_client.chain_id().await.map_err(FundsManagerError::arbitrum)?;

        // Build a custody client
        let gas_sponsor_address =
            Address::from_str(&self.gas_sponsor_address).map_err(FundsManagerError::parse)?;

        let custody_client = CustodyClient::new(
            chain,
            chain_id,
            fireblocks_api_key,
            fireblocks_api_secret,
            self.rpc_url.clone(),
            db_pool.clone(),
            aws_config.clone(),
            gas_sponsor_address,
        )?;

        // Build an execution client
        let execution_client = ExecutionClient::new(
            self.execution_venue_api_key.clone(),
            self.execution_venue_base_url.clone(),
            &self.rpc_url,
        )
        .map_err(FundsManagerError::custom)?;

        // Build a metrics recorder
        let metrics_recorder = MetricsRecorder::new(price_reporter.clone(), &self.rpc_url);

        // Build a fee indexer
        let mut decryption_keys = vec![DecryptionKey::from_hex_str(&self.relayer_decryption_key)
            .map_err(FundsManagerError::parse)?];

        if let Some(protocol_key) = &self.protocol_decryption_key {
            decryption_keys
                .push(DecryptionKey::from_hex_str(protocol_key).map_err(FundsManagerError::parse)?);
        }

        let fee_indexer = Indexer::new(
            chain_id,
            chain,
            aws_config.clone(),
            darkpool_client.clone(),
            decryption_keys,
            db_pool.clone(),
            relayer_client.clone(),
            custody_client.clone(),
            price_reporter,
        );

        let custody_client = Arc::new(custody_client);
        let execution_client = Arc::new(execution_client);
        let metrics_recorder = Arc::new(metrics_recorder);
        let fee_indexer = Arc::new(fee_indexer);

        Ok(ChainClients { custody_client, execution_client, metrics_recorder, fee_indexer })
    }
}

/// Chain-specific clients used by the funds manager, parsed from a
/// configuration object
#[derive(Clone)]
pub struct ChainClients {
    /// The custody client for managing funds on the given chain
    pub(crate) custody_client: Arc<CustodyClient>,
    /// The execution client for executing swaps on the given chain
    pub(crate) execution_client: Arc<ExecutionClient>,
    /// The metrics recorder for the given chain
    // TODO: Turn into top-level chain-agnostic struct that holds references to
    // necessary chain-specific clients
    pub(crate) metrics_recorder: Arc<MetricsRecorder>,
    /// The fee indexer for the given chain
    pub(crate) fee_indexer: Arc<Indexer>,
}
