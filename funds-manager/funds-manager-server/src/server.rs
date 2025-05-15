//! Defines the server which encapsulates all dependencies for funds manager
//! execution

use std::{collections::HashMap, error::Error, sync::Arc};

use aws_config::{BehaviorVersion, Region};
use funds_manager_api::quoters::ExecutionQuote;
use renegade_common::types::{
    chain::Chain,
    hmac::HmacKey,
    token::{Token, USDC_TICKER},
};
use renegade_config::setup_token_remaps;

use crate::{
    cli::{ChainClients, Cli, Environment},
    custody_client::CustodyClient,
    db::create_db_pool,
    error::FundsManagerError,
    execution_client::ExecutionClient,
    metrics::MetricsRecorder,
    relayer_client::RelayerClient,
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
    /// The HMAC key for signing quotes
    pub quote_hmac_key: HmacKey,
    /// The chain clients
    pub chain_clients: HashMap<Chain, ChainClients>,
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

        let hmac_key = args.get_hmac_key();
        let quote_hmac_key = args.get_quote_hmac_key();

        // Create a database connection pool using bb8
        let db_pool = create_db_pool(&args.db_url).await?;
        let arc_pool = Arc::new(db_pool);

        let mut chain_clients = HashMap::new();
        for (chain, config) in chain_configs {
            let usdc_mint = Token::from_ticker_on_chain(USDC_TICKER, chain).get_addr();
            let clients = config
                .build_clients(
                    chain,
                    args.fireblocks_api_key.clone(),
                    args.fireblocks_api_secret.clone(),
                    arc_pool.clone(),
                    aws_config.clone(),
                    &usdc_mint,
                )
                .await?;

            chain_clients.insert(chain, clients);
        }

        Ok(Server { hmac_key, quote_hmac_key, chain_clients, environment: args.environment })
    }

    /// Sign a quote using the quote HMAC key and returns the signature as a
    /// hex string
    pub fn sign_quote(&self, quote: &ExecutionQuote) -> Result<String, FundsManagerError> {
        let canonical_string = quote.to_canonical_string();
        let sig = self.quote_hmac_key.compute_mac(canonical_string.as_bytes());
        let signature = hex::encode(sig);
        Ok(signature)
    }

    /// Get the relayer client for the given chain
    pub fn get_relayer_client(
        &self,
        chain: &Chain,
    ) -> Result<Arc<RelayerClient>, FundsManagerError> {
        self.chain_clients
            .get(chain)
            .map(|clients| clients.relayer_client.clone())
            .ok_or(FundsManagerError::custom(format!("No relayer client configured for {chain}")))
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
