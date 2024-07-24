//! The fee sweeper, sweeps for unredeemed fees in the Renegade protocol and
//! redeems them
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

pub mod custody_client;
pub mod db;
pub mod error;
pub mod fee_indexer;
pub mod handlers;
pub mod relayer_client;

use aws_config::{BehaviorVersion, Region, SdkConfig};
use error::FundsManagerError;
use ethers::signers::LocalWallet;
use fee_indexer::Indexer;
use funds_manager_api::{
    GET_DEPOSIT_ADDRESS_ROUTE, INDEX_FEES_ROUTE, PING_ROUTE, REDEEM_FEES_ROUTE,
    WITHDRAW_CUSTODY_ROUTE,
};
use handlers::{
    get_deposit_address_handler, index_fees_handler, quoter_withdraw_handler, redeem_fees_handler,
};
use relayer_client::RelayerClient;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_util::{err_str, raw_err_str, telemetry::configure_telemetry};

use std::{collections::HashMap, error::Error, str::FromStr, sync::Arc};

use arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use clap::Parser;
use tracing::error;
use warp::{filters::query::query, Filter};

use crate::custody_client::CustodyClient;
use crate::error::ApiError;

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

// -------
// | Cli |
// -------

/// The cli for the fee sweeper
#[rustfmt::skip]
#[derive(Clone, Debug, Parser)]
struct Cli {
    // --- Environment Configs --- //

    /// The URL of the relayer to use
    #[clap(long, env = "RELAYER_URL")]
    relayer_url: String,
    /// The address of the darkpool contract
    #[clap(short = 'a', long, env = "DARKPOOL_ADDRESS")]
    darkpool_address: String,
    /// The chain to redeem fees for
    #[clap(long, default_value = "mainnet", env = "CHAIN")]
    chain: Chain,
    /// The token address of the USDC token, used to get prices for fee
    /// redemption
    #[clap(long, env = "USDC_MINT")]
    usdc_mint: String,

    // --- Decryption Keys --- //

    /// The fee decryption key to use
    #[clap(long, env = "RELAYER_DECRYPTION_KEY")]
    relayer_decryption_key: String,
    /// The fee decryption key to use for the protocol fees
    ///
    /// This argument is not necessary, protocol fee indexing is skipped if this
    /// is omitted
    #[clap(long, env = "PROTOCOL_DECRYPTION_KEY")]
    protocol_decryption_key: Option<String>,

    //  --- Api Secrets --- //

    /// The Arbitrum RPC url to use
    #[clap(short, long, env = "RPC_URL")]
    rpc_url: String,
    /// The database url
    #[clap(long, env = "DATABASE_URL")]
    db_url: String,
    /// The fireblocks api key
    #[clap(long, env = "FIREBLOCKS_API_KEY")]
    fireblocks_api_key: String,
    /// The fireblocks api secret
    #[clap(long, env = "FIREBLOCKS_API_SECRET")]
    fireblocks_api_secret: String,

    // --- Server Config --- //

    /// The port to run the server on
    #[clap(long, default_value = "3000")]
    port: u16,
    /// Whether to enable datadog formatted logs
    #[clap(long, default_value = "false")]
    datadog_logging: bool,
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
    /// The custody client
    pub custody_client: CustodyClient,
    /// The AWS config
    pub aws_config: SdkConfig,
}

impl Server {
    /// Build an indexer
    pub async fn build_indexer(&self) -> Result<Indexer, FundsManagerError> {
        let db_conn = db::establish_connection(&self.db_url)
            .await
            .map_err(err_str!(FundsManagerError::Db))?;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    configure_telemetry(
        cli.datadog_logging, // datadog_enabled
        false,               // otlp_enabled
        false,               // metrics_enabled
        "".to_string(),      // collector_endpoint
        "",                  // statsd_host
        0,                   // statsd_port
    )
    .expect("failed to setup telemetry");

    // Parse an AWS config
    let config = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(DEFAULT_REGION))
        .load()
        .await;

    // Build an Arbitrum client
    let wallet = LocalWallet::from_str(DUMMY_PRIVATE_KEY)?;
    let conf = ArbitrumClientConfig {
        darkpool_addr: cli.darkpool_address,
        chain: cli.chain,
        rpc_url: cli.rpc_url.clone(),
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
    let custody_client =
        CustodyClient::new(cli.fireblocks_api_key, cli.fireblocks_api_secret, cli.rpc_url);
    let server = Server {
        chain_id,
        chain: cli.chain,
        relayer_client: relayer_client.clone(),
        arbitrum_client: client.clone(),
        decryption_keys,
        db_url: cli.db_url,
        custody_client,
        aws_config: config,
    };

    // --- Routes --- //

    let ping = warp::get()
        .and(warp::path(PING_ROUTE))
        .map(|| warp::reply::with_status("PONG", warp::http::StatusCode::OK));

    let index_fees = warp::post()
        .and(warp::path(INDEX_FEES_ROUTE))
        .and(with_server(Arc::new(server.clone())))
        .and_then(index_fees_handler);

    let redeem_fees = warp::post()
        .and(warp::path(REDEEM_FEES_ROUTE))
        .and(with_server(Arc::new(server.clone())))
        .and_then(redeem_fees_handler);

    let withdraw_custody = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(WITHDRAW_CUSTODY_ROUTE))
        .and(warp::body::json())
        .and(with_server(Arc::new(server.clone())))
        .and_then(quoter_withdraw_handler);

    let get_deposit_address = warp::get()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(GET_DEPOSIT_ADDRESS_ROUTE))
        .and(query::<HashMap<String, String>>())
        .and(with_server(Arc::new(server.clone())))
        .and_then(get_deposit_address_handler);

    let routes = ping
        .or(index_fees)
        .or(redeem_fees)
        .or(withdraw_custody)
        .or(get_deposit_address)
        .recover(handle_rejection);
    warp::serve(routes).run(([0, 0, 0, 0], cli.port)).await;

    Ok(())
}

// -----------
// | Helpers |
// -----------

/// Handle a rejection from an endpoint handler
async fn handle_rejection(err: warp::Rejection) -> Result<impl warp::Reply, warp::Rejection> {
    if let Some(api_error) = err.find::<ApiError>() {
        let (code, message) = match api_error {
            ApiError::IndexingError(msg) => (warp::http::StatusCode::BAD_REQUEST, msg),
            ApiError::RedemptionError(msg) => (warp::http::StatusCode::BAD_REQUEST, msg),
            ApiError::InternalError(msg) => (warp::http::StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::BadRequest(msg) => (warp::http::StatusCode::BAD_REQUEST, msg),
        };
        error!("API Error: {:?}", api_error);
        Ok(warp::reply::with_status(message.clone(), code))
    } else {
        error!("Unhandled rejection: {:?}", err);
        Err(err)
    }
}

/// Helper function to clone and pass the server to filters
fn with_server(
    server: Arc<Server>,
) -> impl Filter<Extract = (Arc<Server>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || server.clone())
}
