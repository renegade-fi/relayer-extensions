//! The fee sweeper, sweeps for unredeemed fees in the Renegade protocol and
//! redeems them
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

pub mod db;
pub mod error;
pub mod indexer;
pub mod relayer_client;

use aws_config::{BehaviorVersion, Region, SdkConfig};
use diesel::{pg::PgConnection, Connection};
use error::FundsManagerError;
use ethers::signers::LocalWallet;
use indexer::Indexer;
use relayer_client::RelayerClient;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_util::{
    err_str, raw_err_str,
    telemetry::{setup_system_logger, LevelFilter},
};

use std::{error::Error, str::FromStr, sync::Arc};

use arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use clap::Parser;
use warp::{reply::Json, Filter};

// -------------
// | Constants |
// -------------

/// The block polling interval for the Arbitrum client
const BLOCK_POLLING_INTERVAL_MS: u64 = 100;
/// The default region in which to provision secrets manager secrets
const DEFAULT_REGION: &str = "us-east-2";

// -------
// | Cli |
// -------

/// The cli for the fee sweeper
#[derive(Clone, Debug, Parser)]
struct Cli {
    /// The URL of the relayer to use
    #[clap(long)]
    relayer_url: String,
    /// The Arbitrum RPC url to use
    #[clap(short, long, env = "RPC_URL")]
    rpc_url: String,
    /// The address of the darkpool contract
    #[clap(short = 'a', long)]
    darkpool_address: String,
    /// The chain to redeem fees for
    #[clap(long, default_value = "mainnet")]
    chain: Chain,
    /// The fee decryption key to use
    #[clap(short, long, env = "RELAYER_DECRYPTION_KEY")]
    relayer_decryption_key: String,
    /// The fee decryption key to use for the protocol fees
    ///
    /// This argument is not necessary, protocol fee indexing is skipped if this
    /// is omitted
    #[clap(short, long, env = "PROTOCOL_DECRYPTION_KEY")]
    protocol_decryption_key: Option<String>,
    /// The arbitrum private key used to submit transactions
    #[clap(long = "pkey", env = "ARBITRUM_PRIVATE_KEY")]
    arbitrum_private_key: String,
    /// The database url
    #[clap(long, env = "DATABASE_URL")]
    db_url: String,
    /// The token address of the USDC token, used to get prices for fee
    /// redemption
    #[clap(long)]
    usdc_mint: String,
    /// The port to run the server on
    #[clap(long, default_value = "3000")]
    port: u16,
}

/// The server
#[derive(Clone)]
struct Server {
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
    /// The DB url
    pub db_url: String,
    /// The AWS config
    pub aws_config: SdkConfig,
}

impl Server {
    /// Build an indexer
    pub fn build_indexer(&self) -> Result<Indexer, FundsManagerError> {
        let db_conn =
            PgConnection::establish(&self.db_url).map_err(err_str!(FundsManagerError::Db))?;
        Ok(Indexer::new(
            self.chain_id,
            self.chain,
            self.aws_config.clone(),
            self.arbitrum_client.clone(),
            self.decryption_keys.clone(),
            db_conn,
            self.relayer_client.clone(),
        ))
    }
}

/// Main
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    setup_system_logger(LevelFilter::INFO);
    let cli = Cli::parse();

    // Parse an AWS config
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(DEFAULT_REGION))
        .load()
        .await;

    // Build an Arbitrum client
    let wallet = LocalWallet::from_str(&cli.arbitrum_private_key)?;
    let conf = ArbitrumClientConfig {
        darkpool_addr: cli.darkpool_address,
        chain: cli.chain,
        rpc_url: cli.rpc_url,
        arb_priv_keys: vec![wallet],
        block_polling_interval_ms: BLOCK_POLLING_INTERVAL_MS,
    };
    let client = ArbitrumClient::new(conf).await?;
    let chain_id = client.chain_id().await.map_err(raw_err_str!("Error fetching chain ID: {}"))?;

    // Build the indexer
    let mut decryption_keys = vec![DecryptionKey::from_hex_str(&cli.relayer_decryption_key)?];
    if let Some(protocol_key) = cli.protocol_decryption_key {
        decryption_keys.push(DecryptionKey::from_hex_str(&protocol_key)?);
    }

    let relayer_client = RelayerClient::new(&cli.relayer_url, &cli.usdc_mint);
    let server = Server {
        chain_id,
        chain: cli.chain,
        relayer_client: relayer_client.clone(),
        arbitrum_client: client.clone(),
        decryption_keys,
        db_url: cli.db_url,
        aws_config: config,
    };

    // Define routes
    let ping = warp::get()
        .and(warp::path("ping"))
        .map(|| warp::reply::with_status("PONG", warp::http::StatusCode::OK));

    let routes = ping;
    warp::serve(routes).run(([0, 0, 0, 0], cli.port)).await;

    Ok(())
}
