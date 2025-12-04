//! Defines the indexer struct, a dependency injection container which stores
//! handles to shared resources

use std::sync::Arc;

use alloy::{
    hex,
    primitives::Address,
    providers::{Provider, ProviderBuilder, WsConnect},
};
use darkpool_indexer_api::types::message_queue::Message;
use renegade_common::types::{chain::Chain, hmac::HmacKey};
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;
use tracing::error;

use crate::{
    chain_event_listener::ChainEventListener,
    cli::Cli,
    darkpool_client::DarkpoolClient,
    db::client::DbClient,
    indexer::error::IndexerError,
    message_queue::{DynMessageQueue, MessageQueue, sqs::SqsMessageQueue},
    state_transitions::StateApplicator,
};

mod backfill;
pub mod error;
mod event_indexing;

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
    /// The message queue
    pub message_queue: DynMessageQueue<Message>,
    /// The darkpool client
    pub darkpool_client: DarkpoolClient,
    /// The chain event listener
    pub chain_event_listener: ChainEventListener,
    /// The authentication key for the HTTP API
    pub http_auth_key: Option<HmacKey>,
}

impl Indexer {
    /// Build an indexer from the provided CLI arguments
    pub async fn build_from_cli(cli: &Cli) -> Result<Arc<Self>, IndexerError> {
        cli.configure_telemetry()?;

        // Set up the database client & state applicator
        let db_client = DbClient::new(&cli.database_url).await?;
        let state_applicator = StateApplicator::new(db_client.clone());

        // Set up the message queue client
        let sqs_message_queue =
            SqsMessageQueue::new(cli.sqs_region.clone(), cli.sqs_queue_url.clone()).await;

        let message_queue = DynMessageQueue::new(sqs_message_queue);

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
            message_queue.clone(),
        );

        let http_auth_key = cli
            .auth_key
            .as_ref()
            .map(|key_str| HmacKey::from_base64_string(key_str).map_err(IndexerError::parse))
            .transpose()?;

        let indexer = Self {
            db_client,
            state_applicator,
            message_queue,
            darkpool_client,
            chain_event_listener,
            http_auth_key,
        };

        Ok(Arc::new(indexer))
    }
}

// -------------------
// | Service Helpers |
// -------------------

/// Run the message queue consumer, polling for new messages from the
/// queue and handling them
pub async fn run_message_queue_consumer(indexer: Arc<Indexer>) -> Result<(), IndexerError> {
    loop {
        let message_groups = indexer.message_queue.poll_messages().await?;

        // Process message groups concurrently
        for messages in message_groups.into_values() {
            let indexer_clone = indexer.clone();
            tokio::spawn(async move {
                // Process messages within a message group sequentially
                for (message, deletion_id) in messages {
                    if let Err(e) = indexer_clone.handle_message(message, deletion_id).await {
                        error!("Error handling queue message: {e}")
                    }
                }
            });
        }
    }
}

/// Run the nullifier spend event listener, watching for nullifier spend events
/// and forwarding them to the message queue
pub async fn run_nullifier_spend_listener(indexer: Arc<Indexer>) -> Result<(), IndexerError> {
    indexer.chain_event_listener.watch_nullifiers().await?;
    Ok(())
}

/// Run the recovery ID registration event listener, watching for recovery ID
/// registration events and forwarding them to the message queue
pub async fn run_recovery_id_registration_listener(
    indexer: Arc<Indexer>,
) -> Result<(), IndexerError> {
    indexer.chain_event_listener.watch_recovery_ids().await?;
    Ok(())
}

// ------------------
// | Config Helpers |
// ------------------

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
