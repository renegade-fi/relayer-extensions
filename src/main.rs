//! The fee sweeper, sweeps for unredeemed fees in the Renegade protocol and redeems them
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

pub mod db;
pub mod helpers;
pub mod indexer;

use aws_config::{BehaviorVersion, Region, SdkConfig as AwsConfig};
use diesel::{pg::PgConnection, Connection};
use ethers::signers::LocalWallet;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_util::telemetry::{setup_system_logger, LevelFilter};

use std::{error::Error, str::FromStr};

use arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use clap::Parser;

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
#[derive(Debug, Parser)]
struct Cli {
    /// The URL of the relayer to use
    #[clap(long)]
    relayer_url: String,
    /// The Arbitrum RPC url to use
    #[clap(short, long)]
    rpc_url: String,
    /// The address of the darkpool contract
    #[clap(short = 'a', long)]
    darkpool_address: String,
    /// The chain to redeem fees for
    #[clap(long, default_value = "mainnet")]
    chain: Chain,
    /// The fee decryption key to use
    #[clap(short, long)]
    decryption_key: String,
    /// The arbitrum private key used to submit transactions
    #[clap(long = "pkey")]
    arbitrum_private_key: String,
    /// The database url
    #[clap(long)]
    db_url: String,
    /// The token address of the USDC token, used to get prices for fee redemption
    #[clap(long)]
    usdc_mint: String,
}

impl Cli {
    /// Build a connection to the DB
    pub fn build_db_conn(&self) -> Result<PgConnection, String> {
        PgConnection::establish(&self.db_url).map_err(|e| e.to_string())
    }
}

/// Stores the dependencies needed to index the chain
pub(crate) struct Indexer {
    /// The token address of the USDC token, used to get prices for fee redemption
    pub usdc_mint: String,
    /// The relayer URL
    pub relayer_url: String,
    /// The Arbitrum client
    pub client: ArbitrumClient,
    /// The decryption key
    pub decryption_key: DecryptionKey,
    /// A connection to the DB
    pub db_conn: PgConnection,
    /// The AWS config
    pub aws_config: AwsConfig,
}

impl Indexer {
    /// Constructor
    pub fn new(
        aws_config: AwsConfig,
        client: ArbitrumClient,
        decryption_key: DecryptionKey,
        db_conn: PgConnection,
        usdc_mint: String,
        relayer_url: String,
    ) -> Self {
        Indexer {
            client,
            decryption_key,
            db_conn,
            usdc_mint,
            relayer_url,
            aws_config,
        }
    }
}

/// Main
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    setup_system_logger(LevelFilter::INFO);
    let cli = Cli::parse();
    let db_conn = cli.build_db_conn()?;

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

    // Build the indexer
    let key = DecryptionKey::from_hex_str(&cli.decryption_key)?;
    let mut indexer = Indexer::new(config, client, key, db_conn, cli.usdc_mint, cli.relayer_url);

    // 1. Index all new fees in the DB
    indexer.index_fees().await?;
    // 2. Redeem fees according to the redemption policy
    indexer.redeem_fees().await?;

    Ok(())
}
