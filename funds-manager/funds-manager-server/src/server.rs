//! Defines the server which encapsulates all dependencies for funds manager
//! execution

use std::{error::Error, str::FromStr, sync::Arc};

use aws_config::{BehaviorVersion, Region, SdkConfig};
use ethers::{signers::LocalWallet, types::Address};
use funds_manager_api::quoters::ExecutionQuote;
use renegade_arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_common::types::hmac::HmacKey;
use renegade_config::setup_token_remaps;
use renegade_util::raw_err_str;

use crate::{
    custody_client::CustodyClient,
    db::{create_db_pool, DbPool},
    error::FundsManagerError,
    execution_client::ExecutionClient,
    fee_indexer::Indexer,
    metrics::MetricsRecorder,
    relayer_client::RelayerClient,
    Cli,
};

// -------------
// | Constants |
// -------------

/// The block polling interval for the Arbitrum client
const BLOCK_POLLING_INTERVAL_MS: u64 = 100;
/// The default region in which to provision secrets manager secrets
const DEFAULT_REGION: &str = "us-east-2";
/// The dummy private key used to instantiate the arbitrum client
///
/// We don't need any client functionality using a real private key, so instead
/// we use the key deployed by Arbitrum on local devnets
const DUMMY_PRIVATE_KEY: &str =
    "0xb6b15c8cb491557369f3c7d2c287b053eb229daa9c22138887752191c9520659";

/// The server
#[derive(Clone)]
pub(crate) struct Server {
    /// The id of the chain this indexer targets
    pub chain_id: u64,
    /// The chain this indexer targets
    pub chain: Chain,
    /// A client for interacting with the relayer
    pub relayer_client: RelayerClient,
    /// The Arbitrum client
    pub arbitrum_client: ArbitrumClient,
    /// The decryption key
    pub decryption_keys: Vec<DecryptionKey>,
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The custody client
    pub custody_client: CustodyClient,
    /// The execution client
    pub execution_client: ExecutionClient,
    /// The AWS config
    pub aws_config: SdkConfig,
    /// The HMAC key for custody endpoint authentication
    pub hmac_key: Option<HmacKey>,
    /// The HMAC key for signing quotes
    pub quote_hmac_key: HmacKey,
    /// The metrics recorder
    pub metrics_recorder: MetricsRecorder,
}

impl Server {
    /// Build a server from the CLI
    pub async fn build_from_cli(args: Cli) -> Result<Self, Box<dyn Error>> {
        tokio::task::spawn_blocking(move || {
            setup_token_remaps(None /* token_remap_file */, args.chain)
        })
        .await
        .unwrap()?;

        // Parse an AWS config
        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(DEFAULT_REGION))
            .load()
            .await;

        // Build an Arbitrum client
        let wallet = LocalWallet::from_str(DUMMY_PRIVATE_KEY)?;
        let conf = ArbitrumClientConfig {
            darkpool_addr: args.darkpool_address.clone(),
            chain: args.chain,
            rpc_url: args.rpc_url.clone(),
            arb_priv_keys: vec![wallet],
            block_polling_interval_ms: BLOCK_POLLING_INTERVAL_MS,
        };
        let client = ArbitrumClient::new(conf).await?;
        let chain_id =
            client.chain_id().await.map_err(raw_err_str!("Error fetching chain ID: {}"))?;

        // Build the indexer
        let mut decryption_keys = vec![DecryptionKey::from_hex_str(&args.relayer_decryption_key)?];
        if let Some(protocol_key) = &args.protocol_decryption_key {
            decryption_keys.push(DecryptionKey::from_hex_str(protocol_key)?);
        }

        let hmac_key = args.get_hmac_key();
        let quote_hmac_key = args.get_quote_hmac_key();
        let relayer_client = RelayerClient::new(&args.relayer_url, &args.usdc_mint);

        // Create a database connection pool using bb8
        let db_pool = create_db_pool(&args.db_url).await?;
        let arc_pool = Arc::new(db_pool);

        let gas_sponsor_address = Address::from_str(&args.gas_sponsor_address)?;

        let custody_client = CustodyClient::new(
            args.chain,
            chain_id,
            args.fireblocks_api_key,
            args.fireblocks_api_secret,
            args.rpc_url.clone(),
            arc_pool.clone(),
            config.clone(),
            gas_sponsor_address,
        );

        let execution_client = ExecutionClient::new(
            args.execution_venue_api_key,
            args.execution_venue_base_url,
            &args.rpc_url,
        )?;

        let metrics_recorder = MetricsRecorder::new(relayer_client.clone(), args.rpc_url.clone());

        Ok(Server {
            chain_id,
            chain: args.chain,
            relayer_client: relayer_client.clone(),
            arbitrum_client: client.clone(),
            decryption_keys,
            db_pool: arc_pool,
            custody_client,
            execution_client,
            aws_config: config,
            hmac_key,
            quote_hmac_key,
            metrics_recorder,
        })
    }

    /// Build an indexer
    pub fn build_indexer(&self) -> Result<Indexer, FundsManagerError> {
        Ok(Indexer::new(
            self.chain_id,
            self.chain,
            self.aws_config.clone(),
            self.arbitrum_client.clone(),
            self.decryption_keys.clone(),
            self.db_pool.clone(),
            self.relayer_client.clone(),
            self.custody_client.clone(),
        ))
    }

    /// Sign a quote using the quote HMAC key and returns the signature as a
    /// hex string
    pub fn sign_quote(&self, quote: &ExecutionQuote) -> Result<String, FundsManagerError> {
        let canonical_string = quote.to_canonical_string();
        let sig = self.quote_hmac_key.compute_mac(canonical_string.as_bytes());
        let signature = hex::encode(sig);
        Ok(signature)
    }
}
