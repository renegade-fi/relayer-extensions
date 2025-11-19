//! Defines the indexer struct, a dependency injection container which stores
//! handles to shared resources

use alloy::{
    hex,
    primitives::Address,
    providers::{Provider, ProviderBuilder, WsConnect},
};
use aws_config::Region;
use aws_sdk_sqs::Client as SqsClient;
use renegade_common::types::chain::Chain;
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;
use serde::Serialize;

use crate::{
    chain_event_listener::ChainEventListener, cli::Cli, darkpool_client::DarkpoolClient,
    db::client::DbClient, indexer::error::IndexerError, state_transitions::StateApplicator,
};

mod backfill;
pub mod error;
mod event_indexing;
mod settlement_bundle;

// -------------
// | Constants |
// -------------

/// The address of the darkpool contract deployed on Arbitrum One
const ARBITRUM_ONE_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));
/// The address of the darkpool contract deployed on Arbitrum Sepolia
const ARBITRUM_SEPOLIA_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));
/// The address of the darkpool contract deployed on Base mainnet
const BASE_MAINNET_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));
/// The address of the darkpool contract deployed on Base Sepolia
const BASE_SEPOLIA_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));
/// The address of the darkpool contract deployed on devnet
const DEVNET_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));

/// The block number from which to start listening for nullifier spend events
/// on Arbitrum One
const ARBITRUM_ONE_NULLIFIER_START_BLOCK: u64 = 0;
/// The block number from which to start listening for nullifier spend events
/// on Arbitrum Sepolia
const ARBITRUM_SEPOLIA_NULLIFIER_START_BLOCK: u64 = 0;
/// The block number from which to start listening for nullifier spend events
/// on Base mainnet
const BASE_MAINNET_NULLIFIER_START_BLOCK: u64 = 0;
/// The block number from which to start listening for nullifier spend events
/// on Base Sepolia
const BASE_SEPOLIA_NULLIFIER_START_BLOCK: u64 = 0;
/// The block number from which to start listening for nullifier spend events
/// on devnet
const DEVNET_NULLIFIER_START_BLOCK: u64 = 0;

/// The block number from which to start listening for recovery ID registration
/// events on Arbitrum One
const ARBITRUM_ONE_RECOVERY_ID_START_BLOCK: u64 = 0;
/// The block number from which to start listening for recovery ID registration
/// events on Arbitrum Sepolia
const ARBITRUM_SEPOLIA_RECOVERY_ID_START_BLOCK: u64 = 0;
/// The block number from which to start listening for recovery ID registration
/// events on Base mainnet
const BASE_MAINNET_RECOVERY_ID_START_BLOCK: u64 = 0;
/// The block number from which to start listening for recovery ID registration
/// events on Base Sepolia
const BASE_SEPOLIA_RECOVERY_ID_START_BLOCK: u64 = 0;
/// The block number from which to start listening for recovery ID registration
/// events on devnet
const DEVNET_RECOVERY_ID_START_BLOCK: u64 = 0;

/// The indexer struct. Stores handles to shared resources.
#[derive(Clone)]
pub struct Indexer {
    /// The database client
    pub db_client: DbClient,
    /// The state transition applicator
    pub state_applicator: StateApplicator,
    /// The AWS SQS client
    pub sqs_client: SqsClient,
    /// The URL of the AWS SQS queue
    pub sqs_queue_url: String,
    /// The darkpool client
    pub darkpool_client: DarkpoolClient,
    /// The chain event listener
    pub chain_event_listener: ChainEventListener,
}

impl Indexer {
    /// Build an indexer from the provided CLI arguments
    pub async fn build_from_cli(cli: &Cli) -> Result<Self, IndexerError> {
        // Set up the database client & state applicator
        let db_client = DbClient::new(&cli.database_url).await?;
        let state_applicator = StateApplicator::new(db_client.clone());

        // Set up the AWS SQS client
        let config =
            aws_config::from_env().region(Region::new(cli.sqs_region.clone())).load().await;

        let sqs_client = SqsClient::new(&config);
        let sqs_queue_url = cli.sqs_queue_url.clone();

        // Set up the WebSocket RPC provider & darkpool client
        let ws = WsConnect::new(&cli.ws_rpc_url);
        let ws_provider =
            ProviderBuilder::default().connect_ws(ws).await.map_err(IndexerError::rpc)?.erased();

        let darkpool_address = get_darkpool_address(cli.chain);

        let darkpool = IDarkpoolV2Instance::new(darkpool_address, ws_provider);
        let darkpool_client = DarkpoolClient::new(darkpool);

        // Set up the chain event listener
        let mut conn = db_client.get_db_conn().await?;
        let nullifier_start_block = db_client
            .get_latest_processed_nullifier_block(&mut conn)
            .await?
            .unwrap_or_else(|| get_nullifier_start_block(cli.chain));

        let recovery_id_start_block = db_client
            .get_latest_processed_recovery_id_block(&mut conn)
            .await?
            .unwrap_or_else(|| get_recovery_id_start_block(cli.chain));

        let chain_event_listener = ChainEventListener::new(
            darkpool_client.clone(),
            nullifier_start_block,
            recovery_id_start_block,
            sqs_client.clone(),
            sqs_queue_url.clone(),
        );

        // TODO: Parse remaining CLI arguments

        Ok(Self {
            db_client,
            state_applicator,
            sqs_client,
            sqs_queue_url,
            darkpool_client,
            chain_event_listener,
        })
    }

    /// Send a message to the SQS queue
    pub async fn send_sqs_message<T: Serialize>(
        &self,
        message: T,
        deduplication_id: String,
        message_group_id: String,
    ) -> Result<(), IndexerError> {
        let message_body = serde_json::to_string(&message)?;

        self.sqs_client
            .send_message()
            .queue_url(&self.sqs_queue_url)
            .message_deduplication_id(&deduplication_id)
            .message_group_id(&message_group_id)
            .message_body(message_body)
            .send()
            .await?;

        Ok(())
    }
}

/// Get the darkpool address for the given chain
fn get_darkpool_address(chain: Chain) -> Address {
    match chain {
        Chain::ArbitrumOne => ARBITRUM_ONE_DARKPOOL_ADDRESS,
        Chain::ArbitrumSepolia => ARBITRUM_SEPOLIA_DARKPOOL_ADDRESS,
        Chain::BaseMainnet => BASE_MAINNET_DARKPOOL_ADDRESS,
        Chain::BaseSepolia => BASE_SEPOLIA_DARKPOOL_ADDRESS,
        Chain::Devnet => DEVNET_DARKPOOL_ADDRESS,
    }
}

/// Get the nullifier spend event listener start block for the given chain
fn get_nullifier_start_block(chain: Chain) -> u64 {
    match chain {
        Chain::ArbitrumOne => ARBITRUM_ONE_NULLIFIER_START_BLOCK,
        Chain::ArbitrumSepolia => ARBITRUM_SEPOLIA_NULLIFIER_START_BLOCK,
        Chain::BaseMainnet => BASE_MAINNET_NULLIFIER_START_BLOCK,
        Chain::BaseSepolia => BASE_SEPOLIA_NULLIFIER_START_BLOCK,
        Chain::Devnet => DEVNET_NULLIFIER_START_BLOCK,
    }
}

/// Get the recovery ID registration event listener start block for the given
/// chain
fn get_recovery_id_start_block(chain: Chain) -> u64 {
    match chain {
        Chain::ArbitrumOne => ARBITRUM_ONE_RECOVERY_ID_START_BLOCK,
        Chain::ArbitrumSepolia => ARBITRUM_SEPOLIA_RECOVERY_ID_START_BLOCK,
        Chain::BaseMainnet => BASE_MAINNET_RECOVERY_ID_START_BLOCK,
        Chain::BaseSepolia => BASE_SEPOLIA_RECOVERY_ID_START_BLOCK,
        Chain::Devnet => DEVNET_RECOVERY_ID_START_BLOCK,
    }
}
