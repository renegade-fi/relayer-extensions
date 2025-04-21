//! Defines the core implementation of the on-chain event listener
//! Much of the implementation is borrowed from https://github.com/renegade-fi/renegade/blob/main/workers/chain-events/src/listener.rs

use std::thread::JoinHandle;

use alloy::{
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect},
    rpc::types::Filter,
    sol_types::SolEvent,
};
use ethers::types::TxHash;
use futures_util::StreamExt;
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_arbitrum_client::{
    abi::{NullifierSpent, NullifierSpentFilter},
    client::ArbitrumClient,
    conversion::alloy_u256_to_scalar,
};
use renegade_circuit_types::{
    r#match::{ExternalMatchResult, MatchResult as CircuitMatchResult},
    wallet::Nullifier,
};
use renegade_crypto::fields::u256_to_scalar;
use tracing::{error, info};

use crate::store::{helpers::generate_bundle_id, BundleStore};

use super::error::OnChainEventListenerError;

// ----------
// | Worker |
// ----------

/// The configuration passed to the listener upon startup
#[derive(Clone)]
pub struct OnChainEventListenerConfig {
    /// The ethereum websocket address to use for streaming events
    ///
    /// If not configured, the listener will poll using the arbitrum client
    pub websocket_addr: Option<String>,
    /// An arbitrum client for listening to events
    pub arbitrum_client: ArbitrumClient,
}

impl OnChainEventListenerConfig {
    /// Whether or not a websocket listener is configured
    pub fn has_websocket_listener(&self) -> bool {
        self.websocket_addr.is_some()
    }

    /// Create a new websocket client if available
    pub async fn ws_client(&self) -> Result<DynProvider, OnChainEventListenerError> {
        if !self.has_websocket_listener() {
            panic!("no websocket listener configured");
        }

        // Connect to the websocket
        let addr = self.websocket_addr.clone().unwrap();
        let conn = WsConnect::new(addr);
        let provider = ProviderBuilder::new().on_ws(conn).await?;
        Ok(DynProvider::new(provider))
    }
}

/// The worker responsible for listening for on-chain events, translating them
/// to jobs for other workers, and forwarding these jobs to the relevant workers
pub struct OnChainEventListener {
    /// The executor run in a separate thread
    pub(super) executor: Option<OnChainEventListenerExecutor>,
    /// The thread handle of the executor
    pub(super) executor_handle: Option<JoinHandle<OnChainEventListenerError>>,
}
// ------------
// | Executor |
// ------------

/// The executor that runs in a thread and polls events from on-chain state
#[derive(Clone)]
pub struct OnChainEventListenerExecutor {
    /// A copy of the config that the executor maintains
    config: OnChainEventListenerConfig,
    /// The bundle store to use for retrieving bundle contexts
    bundle_store: BundleStore,
}

impl OnChainEventListenerExecutor {
    /// Create a new executor
    pub fn new(config: OnChainEventListenerConfig, bundle_store: BundleStore) -> Self {
        Self { config, bundle_store }
    }

    /// Shorthand for fetching a reference to the arbitrum client
    fn arbitrum_client(&self) -> &ArbitrumClient {
        &self.config.arbitrum_client
    }

    // --------------
    // | Event Loop |
    // --------------

    /// The main execution loop for the executor
    pub async fn execute(self) -> Result<(), OnChainEventListenerError> {
        // Get the current block number to start from
        let starting_block_number = self
            .arbitrum_client()
            .block_number()
            .await
            .map_err(|err| OnChainEventListenerError::Arbitrum(err.to_string()))?;
        info!("Starting on-chain event listener from current block {starting_block_number}");

        // Begin the watch loop
        let res = self.watch_nullifiers().await.unwrap_err();
        error!("on-chain event listener stream ended unexpectedly: {res}");
        Err(res)
    }

    /// Nullifier watch loop
    async fn watch_nullifiers(&self) -> Result<(), OnChainEventListenerError> {
        if self.config.has_websocket_listener() {
            self.watch_nullifiers_ws().await
        } else {
            self.watch_nullifiers_http().await
        }
    }

    /// Watch for nullifiers via a websocket stream
    async fn watch_nullifiers_ws(&self) -> Result<(), OnChainEventListenerError> {
        info!("listening for nullifiers via websocket");
        // Create the contract instance and the event stream
        let client = self.config.ws_client().await?;
        let contract_addr = self.arbitrum_client().darkpool_alloy_addr();
        let filter = Filter::new().address(contract_addr).event(NullifierSpent::SIGNATURE);
        let mut stream = client.subscribe_logs(&filter).await?.into_stream();

        // Listen for events in a loop
        while let Some(log) = stream.next().await {
            let hash = log
                .transaction_hash
                .ok_or_else(|| OnChainEventListenerError::arbitrum("no tx hash"))?;
            let tx_hash = TxHash::from(hash.0);

            let event = log.log_decode::<NullifierSpent>()?;
            let nullifier = alloy_u256_to_scalar(event.data().nullifier);
            self.handle_nullifier_spent(tx_hash, nullifier).await?;
        }

        unreachable!()
    }

    /// Watch for nullifiers via HTTP polling
    async fn watch_nullifiers_http(&self) -> Result<(), OnChainEventListenerError> {
        info!("listening for nullifiers via HTTP polling");
        // Build a filtered stream on events that the chain-events worker listens for
        let filter = self.arbitrum_client().get_darkpool_client().event::<NullifierSpentFilter>();
        let mut event_stream =
            filter.stream_with_meta().await.map_err(OnChainEventListenerError::arbitrum)?;

        // Listen for events in a loop
        while let Some(res) = event_stream.next().await {
            let (event, meta) = res.map_err(OnChainEventListenerError::arbitrum)?;
            let nullifier = u256_to_scalar(&event.nullifier);
            self.handle_nullifier_spent(meta.transaction_hash, nullifier).await?;
        }

        unreachable!()
    }

    // ----------------------
    // | Nullifier Handlers |
    // ----------------------

    /// Handle a nullifier spent event
    async fn handle_nullifier_spent(
        &self,
        tx: TxHash,
        nullifier: Nullifier,
    ) -> Result<(), OnChainEventListenerError> {
        self.check_external_match_settlement(nullifier, tx).await
    }

    /// Check for an external match settlement on the given transaction hash. If
    /// one is present, record metrics for it
    ///
    /// Returns whether the tx settled an external match
    async fn check_external_match_settlement(
        &self,
        nullifier: Nullifier,
        tx: TxHash,
    ) -> Result<(), OnChainEventListenerError> {
        let matches = self.arbitrum_client().find_external_matches_in_tx(tx).await?;
        for match_result in matches {
            let circuit_match_result: CircuitMatchResult = match_result.try_into().unwrap();
            let external_match_result: ExternalMatchResult = circuit_match_result.into();
            let match_result: ApiExternalMatchResult = external_match_result.into();
            let bundle_id = generate_bundle_id(&match_result, &nullifier).unwrap();
            let bundle_ctx = self.bundle_store.read(&bundle_id).await?;
            if let Some(bundle_ctx) = bundle_ctx {
                self.record_settlement_metrics(&bundle_ctx, &match_result);
            }
        }
        Ok(())
    }
}
