//! Defines the core implementation of the on-chain event listener
//! Much of the implementation is borrowed from https://github.com/renegade-fi/renegade/blob/main/workers/chain-events/src/listener.rs
use std::{sync::Arc, thread::JoinHandle};

use ethers::{
    prelude::StreamExt,
    providers::{Provider, Ws},
    types::H256 as TxHash,
};
use renegade_arbitrum_client::{
    abi::{DarkpoolContract, NullifierSpentFilter},
    client::ArbitrumClient,
};
use tracing::{error, info};

use super::error::OnChainEventListenerError;

// ----------
// | Worker |
// ----------

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
    pub async fn ws_client(
        &self,
    ) -> Result<DarkpoolContract<Provider<Ws>>, OnChainEventListenerError> {
        if !self.has_websocket_listener() {
            panic!("no websocket listener configured");
        }

        // Connect to the websocket
        let addr = self.websocket_addr.clone().unwrap();
        let client = Ws::connect(&addr).await?;
        let provider = Provider::<Ws>::new(client);

        // Create the contract instance
        let contract_addr = self.arbitrum_client.get_darkpool_client().address();
        let contract = DarkpoolContract::new(contract_addr, Arc::new(provider));
        Ok(contract)
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
}

impl OnChainEventListenerExecutor {
    /// Create a new executor
    pub fn new(config: OnChainEventListenerConfig) -> Self {
        Self { config }
    }

    /// Shorthand for fetching a reference to the arbitrum client
    pub fn arbitrum_client(&self) -> &ArbitrumClient {
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
        let contract = self.config.ws_client().await?;
        let filter = contract.event::<NullifierSpentFilter>();
        let mut stream = filter.stream_with_meta().await?;

        // Listen for events in a loop
        while let Some(res) = stream.next().await {
            let (_, meta) = res.map_err(OnChainEventListenerError::arbitrum)?;
            self.handle_nullifier_spent(meta.transaction_hash).await?;
        }

        todo!()
    }

    /// Watch for nullifiers via HTTP polling
    async fn watch_nullifiers_http(&self) -> Result<(), OnChainEventListenerError> {
        info!("listening for nullifiers via HTTP polling");
        // Build a filtered stream on events that the chain-events worker listens for
        let filter = self.arbitrum_client().get_darkpool_client().event::<NullifierSpentFilter>();
        let mut event_stream = filter.stream_with_meta().await?;

        // Listen for events in a loop
        while let Some(res) = event_stream.next().await {
            let (_, meta) = res.map_err(OnChainEventListenerError::arbitrum)?;
            self.handle_nullifier_spent(meta.transaction_hash).await?;
        }

        unreachable!()
    }

    // ----------------------
    // | Nullifier Handlers |
    // ----------------------

    /// Handle a nullifier spent event
    async fn handle_nullifier_spent(&self, tx: TxHash) -> Result<(), OnChainEventListenerError> {
        self.check_external_match_settlement(tx).await?;
        Ok(())
    }

    /// Check for an external match settlement on the given transaction hash. If
    /// one is present, record metrics for it
    ///
    /// Returns whether the tx settled an external match
    async fn check_external_match_settlement(
        &self,
        tx: TxHash,
    ) -> Result<bool, OnChainEventListenerError> {
        let matches = self.arbitrum_client().find_external_matches_in_tx(tx).await?;
        let external_match = !matches.is_empty();

        // Record metrics for each match
        for _match_result in matches {
            // TODO: Record match_result
        }

        Ok(external_match)
    }
}
