//! Defines the server which encapsulates all dependencies for funds manager
//! execution

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    str::FromStr,
    sync::Arc,
};

use aws_config::{BehaviorVersion, Region};
use price_reporter_client::{PriceReporterClient, PriceReporterClientConfig};
use renegade_common::types::{chain::Chain, hmac::HmacKey};
use renegade_config::setup_token_remaps;

use crate::{
    cli::{ChainClients, Cli, Environment},
    custody_client::CustodyClient,
    db::create_db_pool,
    error::FundsManagerError,
    execution_client::ExecutionClient,
    metrics::MetricsRecorder,
    Indexer,
};

// -------------
// | Constants |
// -------------

/// The default region in which to provision secrets manager secrets
const DEFAULT_REGION: &str = "us-east-2";

/// The server
#[derive(Clone)]
pub(crate) struct Server {
    /// The chain-agnostic environment the server is running in
    pub environment: Environment,
    /// The HMAC key for custody endpoint authentication
    pub hmac_key: Option<HmacKey>,
    /// The chain clients
    pub chain_clients: HashMap<Chain, ChainClients>,
    /// The price reporter client
    pub price_reporter: PriceReporterClient,
    /// Chains for which relayer-backed routes should short-circuit instead
    /// of attempting to call the v1 relayer. Populated from the
    /// `DISABLED_RELAYER_CHAINS` env var on startup; used by fee-indexer
    /// handlers to avoid repeatedly hammering a wound-down relayer with
    /// requests it can never serve.
    pub disabled_relayer_chains: HashSet<Chain>,
}

impl Server {
    /// Build a server from the CLI
    pub async fn build_from_cli(args: Cli) -> Result<Self, Box<dyn Error>> {
        // Parse an AWS config
        let aws_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(DEFAULT_REGION))
            .load()
            .await;

        let chain_configs = args.parse_chain_configs(&aws_config).await?;

        for chain in chain_configs.keys() {
            let chain = *chain;
            tokio::task::spawn_blocking(move || {
                setup_token_remaps(None /* token_remap_file */, chain)
            })
            .await
            .unwrap()?;
        }

        let price_reporter = PriceReporterClient::new(PriceReporterClientConfig {
            base_url: args.price_reporter_url.clone(),
            ..Default::default()
        })?;

        let hmac_key = args.get_hmac_key();

        // Create a database connection pool using bb8
        let db_pool = create_db_pool(&args.db_url).await?;
        let arc_pool = Arc::new(db_pool);

        let mut chain_clients = HashMap::new();
        for (chain, config) in chain_configs {
            let clients = config
                .build_clients(
                    chain,
                    args.fireblocks_api_key.clone(),
                    args.fireblocks_api_secret.clone(),
                    arc_pool.clone(),
                    aws_config.clone(),
                    price_reporter.clone(),
                )
                .await?;

            chain_clients.insert(chain, clients);
        }

        let disabled_relayer_chains = parse_disabled_relayer_chains(
            args.disabled_relayer_chains.as_deref(),
        )?;

        Ok(Server {
            hmac_key,
            chain_clients,
            environment: args.environment,
            price_reporter,
            disabled_relayer_chains,
        })
    }

    /// Whether the relayer-backed routes for this chain should short-circuit
    /// because the chain's relayer has been wound down.
    pub fn is_relayer_disabled(&self, chain: &Chain) -> bool {
        self.disabled_relayer_chains.contains(chain)
    }

    /// Get the custody client for the given chain
    pub fn get_custody_client(
        &self,
        chain: &Chain,
    ) -> Result<Arc<CustodyClient>, FundsManagerError> {
        self.chain_clients
            .get(chain)
            .map(|clients| clients.custody_client.clone())
            .ok_or(FundsManagerError::custom(format!("No custody client configured for {chain}")))
    }

    /// Get the execution client for the given chain
    pub fn get_execution_client(
        &self,
        chain: &Chain,
    ) -> Result<Arc<ExecutionClient>, FundsManagerError> {
        self.chain_clients
            .get(chain)
            .map(|clients| clients.execution_client.clone())
            .ok_or(FundsManagerError::custom(format!("No execution client configured for {chain}")))
    }

    /// Get the metrics recorder for the given chain
    pub fn get_metrics_recorder(
        &self,
        chain: &Chain,
    ) -> Result<Arc<MetricsRecorder>, FundsManagerError> {
        self.chain_clients
            .get(chain)
            .map(|clients| clients.metrics_recorder.clone())
            .ok_or(FundsManagerError::custom(format!("No metrics recorder configured for {chain}")))
    }

    /// Get the fee indexer for the given chain
    pub fn get_fee_indexer(&self, chain: &Chain) -> Result<Arc<Indexer>, FundsManagerError> {
        self.chain_clients
            .get(chain)
            .map(|clients| clients.fee_indexer.clone())
            .ok_or(FundsManagerError::custom(format!("No fee indexer configured for {chain}")))
    }
}

/// Parse a comma-separated `Chain` list (e.g. "arbitrum-one,base-mainnet")
/// into a `HashSet`. `None` and empty strings yield an empty set; any
/// non-empty entry that fails `Chain::from_str` is a startup-fatal error.
fn parse_disabled_relayer_chains(raw: Option<&str>) -> Result<HashSet<Chain>, FundsManagerError> {
    let Some(raw) = raw else { return Ok(HashSet::new()) };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            Chain::from_str(s).map_err(|e| {
                FundsManagerError::custom(format!(
                    "Invalid Chain in DISABLED_RELAYER_CHAINS ({s}): {e:?}"
                ))
            })
        })
        .collect()
}
