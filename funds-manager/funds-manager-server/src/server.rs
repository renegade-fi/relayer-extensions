//! Defines the server which encapsulates all dependencies for funds manager
//! execution

use std::{error::Error, str::FromStr, sync::Arc, time::Duration};

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::Address;
use aws_config::{BehaviorVersion, Region, SdkConfig};
use funds_manager_api::quoters::ExecutionQuote;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_common::types::{chain::Chain, hmac::HmacKey, token::Token};
use renegade_config::setup_token_remaps;
use renegade_darkpool_client::{
    client::{DarkpoolClientConfig, DarkpoolClientInner},
    traits::DarkpoolImpl,
};

use crate::{
    cli::{ChainConfig, ChainConfigsMap, Cli, Environment},
    custody_client::CustodyClient,
    db::{create_db_pool, DbPool},
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

/// The block polling interval for the darkpool client
const BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

/// The error message emitted when an unsupported chain is requested.
const ERR_UNSUPPORTED_CHAIN: &str = "Unsupported chain";

/// The server
#[derive(Clone)]
pub(crate) struct Server {
    /// The chain-agnostic environment the server is running in
    pub environment: Environment,
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The AWS config
    pub aws_config: SdkConfig,
    /// The fireblocks API key
    pub fireblocks_api_key: String,
    /// The fireblocks API secret
    pub fireblocks_api_secret: String,
    /// The HMAC key for custody endpoint authentication
    pub hmac_key: Option<HmacKey>,
    /// The HMAC key for signing quotes
    pub quote_hmac_key: HmacKey,
    /// The chain configs
    pub chain_configs: ChainConfigsMap,
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
        let db_pool = Arc::new(create_db_pool(&args.db_url).await?);

        Ok(Server {
            environment: args.environment,
            db_pool,
            aws_config,
            fireblocks_api_key: args.fireblocks_api_key,
            fireblocks_api_secret: args.fireblocks_api_secret,
            hmac_key,
            quote_hmac_key,
            chain_configs,
        })
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
    pub fn get_relayer_client(&self, chain: Chain) -> Result<RelayerClient, FundsManagerError> {
        let chain_config = self.get_chain_config(&chain)?;

        let usdc_mint = Token::usdc().get_addr();
        Ok(RelayerClient::new(&chain_config.relayer_url, &usdc_mint))
    }

    /// Get the custody client for the given chain
    pub fn get_custody_client(&self, chain: Chain) -> Result<CustodyClient, FundsManagerError> {
        let chain_config = self.get_chain_config(&chain)?;

        let gas_sponsor_address = Address::from_str(&chain_config.gas_sponsor_address)
            .map_err(FundsManagerError::parse)?;

        CustodyClient::new(
            chain,
            chain_config.chain_id,
            self.fireblocks_api_key.clone(),
            self.fireblocks_api_secret.clone(),
            chain_config.rpc_url.clone(),
            self.db_pool.clone(),
            self.aws_config.clone(),
            gas_sponsor_address,
        )
    }

    /// Get the execution client for the given chain
    pub fn get_execution_client(&self, chain: Chain) -> Result<ExecutionClient, FundsManagerError> {
        let chain_config = self.get_chain_config(&chain)?;

        ExecutionClient::new(
            chain_config.execution_venue_api_key.clone(),
            chain_config.execution_venue_base_url.clone(),
            &chain_config.rpc_url,
        )
        .map_err(FundsManagerError::custom)
    }

    /// Get the metrics recorder for the given chain
    pub fn get_metrics_recorder(&self, chain: Chain) -> Result<MetricsRecorder, FundsManagerError> {
        let chain_config = self.get_chain_config(&chain)?;
        let relayer_client = self.get_relayer_client(chain)?;
        Ok(MetricsRecorder::new(relayer_client, &chain_config.rpc_url))
    }

    /// Get the fee indexer for the given chain
    pub fn get_fee_indexer<D: DarkpoolImpl>(
        &self,
        chain: Chain,
    ) -> Result<Indexer<D>, FundsManagerError> {
        let chain_config = self.get_chain_config(&chain)?;

        let mut decryption_keys =
            vec![DecryptionKey::from_hex_str(&chain_config.relayer_decryption_key)
                .map_err(FundsManagerError::parse)?];

        if let Some(protocol_key) = &chain_config.protocol_decryption_key {
            decryption_keys
                .push(DecryptionKey::from_hex_str(protocol_key).map_err(FundsManagerError::parse)?);
        }

        Ok(Indexer::new(
            chain_config.chain_id,
            chain,
            self.aws_config.clone(),
            self.get_darkpool_client::<D>(chain)?,
            decryption_keys,
            self.db_pool.clone(),
            self.get_relayer_client(chain)?,
            self.get_custody_client(chain)?,
        ))
    }

    // -------------------
    // | Private Helpers |
    // -------------------

    /// Get the chain config for the given chain
    fn get_chain_config(&self, chain: &Chain) -> Result<&ChainConfig, FundsManagerError> {
        self.chain_configs.get(chain).ok_or(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN))
    }

    /// Get the darkpool client for the given chain, generic over the Darkpool
    /// implementation
    fn get_darkpool_client<D: DarkpoolImpl>(
        &self,
        chain: Chain,
    ) -> Result<DarkpoolClientInner<D>, FundsManagerError> {
        let chain_config = self
            .chain_configs
            .get(&chain)
            .ok_or(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN))?;

        let private_key = PrivateKeySigner::random();
        let conf = DarkpoolClientConfig {
            darkpool_addr: chain_config.darkpool_address.clone(),
            chain,
            rpc_url: chain_config.rpc_url.clone(),
            private_key,
            block_polling_interval: BLOCK_POLLING_INTERVAL,
        };

        DarkpoolClientInner::<D>::new(conf).map_err(FundsManagerError::custom)
    }
}

// ----------
// | Macros |
// ----------

/// Macro for executing a function with a fee indexer for the given chain
#[macro_export]
macro_rules! with_fee_indexer {
    ($server:expr, $chain:expr, $func:expr) => {
        match $chain {
            Chain::ArbitrumOne | Chain::ArbitrumSepolia => {
                let indexer = $server.get_fee_indexer::<ArbitrumDarkpool>($chain)?;
                $func(indexer).await
            },
            Chain::BaseMainnet | Chain::BaseSepolia => {
                let indexer = $server.get_fee_indexer::<BaseDarkpool>($chain)?;
                $func(indexer).await
            },
            _ => Err(FundsManagerError::custom(ERR_UNSUPPORTED_CHAIN)),
        }
    };
}
