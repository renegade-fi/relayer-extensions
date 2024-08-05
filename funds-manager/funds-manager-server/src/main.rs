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
pub mod helpers;
pub mod middleware;
pub mod relayer_client;

use aws_config::{BehaviorVersion, Region, SdkConfig};
use db::{create_db_pool, DbPool};
use error::FundsManagerError;
use ethers::signers::LocalWallet;
use fee_indexer::Indexer;
use funds_manager_api::{
    CreateHotWalletRequest, RegisterGasWalletRequest, ReportActivePeersRequest,
    TransferToVaultRequest, WithdrawFeeBalanceRequest, WithdrawGasRequest,
    WithdrawToHotWalletRequest, GET_DEPOSIT_ADDRESS_ROUTE, GET_FEE_WALLETS_ROUTE, INDEX_FEES_ROUTE,
    PING_ROUTE, REDEEM_FEES_ROUTE, REGISTER_GAS_WALLET_ROUTE, REPORT_ACTIVE_PEERS_ROUTE,
    TRANSFER_TO_VAULT_ROUTE, WITHDRAW_CUSTODY_ROUTE, WITHDRAW_FEE_BALANCE_ROUTE,
    WITHDRAW_GAS_ROUTE, WITHDRAW_TO_HOT_WALLET_ROUTE,
};
use handlers::{
    create_gas_wallet_handler, create_hot_wallet_handler, get_deposit_address_handler,
    get_fee_wallets_handler, get_hot_wallet_balances_handler, index_fees_handler,
    quoter_withdraw_handler, redeem_fees_handler, register_gas_wallet_handler,
    report_active_peers_handler, transfer_to_vault_handler, withdraw_fee_balance_handler,
    withdraw_from_vault_handler, withdraw_gas_handler,
};
use middleware::{identity, with_hmac_auth, with_json_body};
use relayer_client::RelayerClient;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_util::{raw_err_str, telemetry::configure_telemetry};

use std::{collections::HashMap, error::Error, str::FromStr, sync::Arc};

use arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use clap::Parser;
use funds_manager_api::WithdrawFundsRequest;
use tracing::{error, warn};
use warp::Filter;

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
#[derive(Parser)]
#[clap(about = "Funds manager server")]
struct Cli {
    // --- Authentication --- //

    /// The HMAC key to use for authentication
    #[clap(long, conflicts_with = "disable_auth", env = "HMAC_KEY")]
    hmac_key: Option<String>,
    /// Whether to disable authentication
    #[clap(long, conflicts_with = "hmac_key")]
    disable_auth: bool,

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

impl Cli {
    /// Validate the CLI arguments
    fn validate(&self) -> Result<(), String> {
        if self.hmac_key.is_none() && !self.disable_auth {
            Err("Either --hmac-key or --disable-auth must be provided".to_string())
        } else {
            Ok(())
        }
    }

    /// Get the HMAC key as a 32-byte array
    fn get_hmac_key(&self) -> Option<[u8; 32]> {
        self.hmac_key.as_ref().map(|key| {
            let decoded = hex::decode(key).expect("Invalid HMAC key");
            if decoded.len() != 32 {
                panic!("HMAC key must be 32 bytes long");
            }
            let mut array = [0u8; 32];
            array.copy_from_slice(&decoded);
            array
        })
    }
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
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The custody client
    pub custody_client: CustodyClient,
    /// The AWS config
    pub aws_config: SdkConfig,
    /// The HMAC key for custody endpoint authentication
    pub hmac_key: Option<[u8; 32]>,
}

impl Server {
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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    cli.validate()?;
    if cli.hmac_key.is_none() {
        warn!("Authentication is disabled. This is not recommended for production use.");
    }

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
        darkpool_addr: cli.darkpool_address.clone(),
        chain: cli.chain,
        rpc_url: cli.rpc_url.clone(),
        arb_priv_keys: vec![wallet],
        block_polling_interval_ms: BLOCK_POLLING_INTERVAL_MS,
    };
    let client = ArbitrumClient::new(conf).await?;
    let chain_id = client.chain_id().await.map_err(raw_err_str!("Error fetching chain ID: {}"))?;

    // Build the indexer
    let mut decryption_keys = vec![DecryptionKey::from_hex_str(&cli.relayer_decryption_key)?];
    if let Some(protocol_key) = &cli.protocol_decryption_key {
        decryption_keys.push(DecryptionKey::from_hex_str(protocol_key)?);
    }

    let hmac_key = cli.get_hmac_key();
    let relayer_client = RelayerClient::new(&cli.relayer_url, &cli.usdc_mint);

    // Create a database connection pool using bb8
    let db_pool = create_db_pool(&cli.db_url).await?;
    let arc_pool = Arc::new(db_pool);

    let custody_client = CustodyClient::new(
        chain_id,
        cli.fireblocks_api_key,
        cli.fireblocks_api_secret,
        cli.rpc_url,
        arc_pool.clone(),
        config.clone(),
    );

    let server = Server {
        chain_id,
        chain: cli.chain,
        relayer_client: relayer_client.clone(),
        arbitrum_client: client.clone(),
        decryption_keys,
        db_pool: arc_pool,
        custody_client,
        aws_config: config,
        hmac_key,
    };

    // ----------
    // | Routes |
    // ----------

    let server = Arc::new(server);
    let ping = warp::get()
        .and(warp::path(PING_ROUTE))
        .map(|| warp::reply::with_status("PONG", warp::http::StatusCode::OK));

    // --- Fee Indexing --- //

    let index_fees = warp::post()
        .and(warp::path("fees"))
        .and(warp::path(INDEX_FEES_ROUTE))
        .and(with_server(server.clone()))
        .and_then(index_fees_handler);

    let redeem_fees = warp::post()
        .and(warp::path("fees"))
        .and(warp::path(REDEEM_FEES_ROUTE))
        .and(with_server(server.clone()))
        .and_then(redeem_fees_handler);

    let get_balances = warp::get()
        .and(warp::path("fees"))
        .and(warp::path(GET_FEE_WALLETS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(get_fee_wallets_handler);

    let withdraw_fee_balance = warp::post()
        .and(warp::path("fees"))
        .and(warp::path(WITHDRAW_FEE_BALANCE_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawFeeBalanceRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(withdraw_fee_balance_handler);

    // --- Quoters --- //

    let withdraw_custody = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(WITHDRAW_CUSTODY_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawFundsRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(quoter_withdraw_handler);

    let get_deposit_address = warp::get()
        .and(warp::path("custody"))
        .and(warp::path("quoters"))
        .and(warp::path(GET_DEPOSIT_ADDRESS_ROUTE))
        .and(with_server(server.clone()))
        .and_then(get_deposit_address_handler);

    // --- Gas --- //

    let withdraw_gas = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas"))
        .and(warp::path(WITHDRAW_GAS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawGasRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(withdraw_gas_handler);

    let add_gas_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas-wallets"))
        .and(with_hmac_auth(server.clone()))
        .and(with_server(server.clone()))
        .and_then(create_gas_wallet_handler);

    let register_gas_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas-wallets"))
        .and(warp::path(REGISTER_GAS_WALLET_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<RegisterGasWalletRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(register_gas_wallet_handler);

    let report_active_peers = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("gas-wallets"))
        .and(warp::path(REPORT_ACTIVE_PEERS_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<ReportActivePeersRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(report_active_peers_handler);

    // --- Hot Wallets --- //

    let create_hot_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<CreateHotWalletRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(create_hot_wallet_handler);

    let get_hot_wallet_balances = warp::get()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(with_hmac_auth(server.clone()))
        .and(warp::query::<HashMap<String, String>>())
        .and(with_server(server.clone()))
        .and_then(get_hot_wallet_balances_handler);

    let transfer_to_vault = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(warp::path(TRANSFER_TO_VAULT_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<TransferToVaultRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(transfer_to_vault_handler);

    let transfer_to_hot_wallet = warp::post()
        .and(warp::path("custody"))
        .and(warp::path("hot-wallets"))
        .and(warp::path(WITHDRAW_TO_HOT_WALLET_ROUTE))
        .and(with_hmac_auth(server.clone()))
        .map(with_json_body::<WithdrawToHotWalletRequest>)
        .and_then(identity)
        .and(with_server(server.clone()))
        .and_then(withdraw_from_vault_handler);

    let routes = ping
        .or(index_fees)
        .or(redeem_fees)
        .or(withdraw_custody)
        .or(get_deposit_address)
        .or(withdraw_gas)
        .or(report_active_peers)
        .or(register_gas_wallet)
        .or(add_gas_wallet)
        .or(get_balances)
        .or(withdraw_fee_balance)
        .or(transfer_to_vault)
        .or(transfer_to_hot_wallet)
        .or(get_hot_wallet_balances)
        .or(create_hot_wallet)
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
            ApiError::Unauthenticated(msg) => (warp::http::StatusCode::UNAUTHORIZED, msg),
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
