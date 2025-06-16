//! Defines the core implementation of the on-chain event listener
//! Much of the implementation is borrowed from https://github.com/renegade-fi/renegade/blob/main/workers/chain-events/src/listener.rs

use std::{sync::Arc, thread::JoinHandle};

use alloy::{
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect},
    rpc::types::{trace::geth::CallFrame, Filter},
    sol_types::SolEvent,
};
use alloy_primitives::TxHash;
use futures_util::StreamExt;
use price_reporter_client::PriceReporterClient;
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_circuit_types::wallet::Nullifier;
use renegade_common::types::chain::Chain;
use renegade_darkpool_client::{
    conversion::u256_to_scalar, traits::DarkpoolImpl, DarkpoolClient, DarkpoolImplementation,
};
use tracing::{error, info};

use crate::{bundle_store::BundleStore, chain_events::abis::parse_external_match};
use crate::{
    chain_events::abis::ExternalMatch,
    server::{
        gas_estimation::gas_cost_sampler::GasCostSampler, rate_limiter::AuthServerRateLimiter,
    },
};

use super::error::OnChainEventListenerError;

/// The nullifier spent event for the darkpool
type NullifierSpent = <DarkpoolImplementation as DarkpoolImpl>::NullifierSpent;

// ----------
// | Worker |
// ----------

/// The configuration passed to the listener upon startup
#[derive(Clone)]
pub struct OnChainEventListenerConfig {
    /// The ethereum websocket address to use for streaming events
    ///
    /// If not configured, the listener will poll using the darkpool client
    pub websocket_addr: Option<String>,
    /// A darkpool client for listening to events
    pub darkpool_client: DarkpoolClient,
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
        let provider = ProviderBuilder::new().connect_ws(conn).await?;
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
    /// The chain for which the executor is configured
    pub(crate) chain: Chain,
    /// The rate limiter
    pub(crate) rate_limiter: AuthServerRateLimiter,
    /// The price reporter client with WebSocket streaming support
    pub(crate) price_reporter_client: Arc<PriceReporterClient>,
    /// The gas cost sampler
    pub(crate) gas_cost_sampler: Arc<GasCostSampler>,
}

impl OnChainEventListenerExecutor {
    /// Create a new executor
    pub fn new(
        config: OnChainEventListenerConfig,
        bundle_store: BundleStore,
        rate_limiter: AuthServerRateLimiter,
        price_reporter_client: Arc<PriceReporterClient>,
        gas_cost_sampler: Arc<GasCostSampler>,
        chain: Chain,
    ) -> Self {
        Self { config, bundle_store, rate_limiter, price_reporter_client, gas_cost_sampler, chain }
    }

    /// Shorthand for fetching a reference to the darkpool client
    fn darkpool_client(&self) -> &DarkpoolClient {
        &self.config.darkpool_client
    }

    // --------------
    // | Event Loop |
    // --------------

    /// The main execution loop for the executor
    pub async fn execute(self) -> Result<(), OnChainEventListenerError> {
        // Get the current block number to start from
        let starting_block_number = self
            .darkpool_client()
            .block_number()
            .await
            .map_err(|err| OnChainEventListenerError::Darkpool(err.to_string()))?;
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
        let contract_addr = self.darkpool_client().darkpool_addr();
        let filter = Filter::new().address(contract_addr).event(NullifierSpent::SIGNATURE);
        let mut stream = client.subscribe_logs(&filter).await?.into_stream();

        // Listen for events in a loop
        while let Some(log) = stream.next().await {
            let tx_hash = log
                .transaction_hash
                .ok_or_else(|| OnChainEventListenerError::darkpool("no tx hash"))?;

            let event = log.log_decode::<NullifierSpent>()?;
            let nullifier = u256_to_scalar(event.data().nullifier);
            self.handle_nullifier_spent(tx_hash, nullifier);
        }

        unreachable!()
    }

    /// Watch for nullifiers via HTTP polling
    async fn watch_nullifiers_http(&self) -> Result<(), OnChainEventListenerError> {
        info!("listening for nullifiers via HTTP polling");
        // Build a filtered stream on events that the chain-events worker listens for
        let filter = self.darkpool_client().event_filter::<NullifierSpent>();
        let mut event_stream =
            filter.subscribe().await.map_err(OnChainEventListenerError::darkpool)?.into_stream();

        // Listen for events in a loop
        while let Some(res) = event_stream.next().await {
            let (event, meta) = res.map_err(OnChainEventListenerError::darkpool)?;
            let tx_hash = meta
                .transaction_hash
                .ok_or_else(|| OnChainEventListenerError::darkpool("no tx hash"))?;
            let nullifier = u256_to_scalar(event.nullifier);

            self.handle_nullifier_spent(tx_hash, nullifier);
        }

        unreachable!()
    }

    // ----------------------
    // | Nullifier Handlers |
    // ----------------------

    /// Handle a nullifier spent event
    fn handle_nullifier_spent(&self, tx: TxHash, nullifier: Nullifier) {
        let self_clone = self.clone();
        tokio::spawn(async move {
            let res = self_clone.check_external_match_settlement(nullifier, tx).await;
            if let Err(e) = res {
                error!("failed to check external match settlement: {e}");
            }
        });
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
        let matches = self.fetch_external_matches_in_tx(tx).await?;
        for external_match in matches {
            let bundle_id = external_match.bundle_id(&nullifier)?;
            if let Some(bundle_ctx) = self.bundle_store.read(&bundle_id).await? {
                // Increase rate limit
                self.add_bundle_rate_limit_token(
                    bundle_ctx.key_description.clone(),
                    bundle_ctx.shared,
                )
                .await;

                // Record settlement metrics
                let api_match: ApiExternalMatchResult = external_match.match_result().into();
                self.record_settlement_metrics(&bundle_ctx, &api_match);

                // Record sponsorship metrics
                if let Some(gas_sponsorship_info) = &bundle_ctx.gas_sponsorship_info {
                    self.record_settled_match_sponsorship(
                        &bundle_ctx,
                        &api_match,
                        gas_sponsorship_info,
                    )
                    .await?;
                }

                // Cleanup the bundle context
                self.bundle_store.cleanup_by_nullifier(&bundle_ctx.nullifier).await?;

                // Record settlement delay
                self.record_settlement_delay(tx, &bundle_ctx, self.darkpool_client()).await?;
            }
        }

        Ok(())
    }

    /// Fetch all external matches in a transaction
    async fn fetch_external_matches_in_tx(
        &self,
        tx: TxHash,
    ) -> Result<Vec<ExternalMatch>, OnChainEventListenerError> {
        let darkpool_calls: Vec<CallFrame> =
            self.darkpool_client().fetch_tx_darkpool_calls(tx).await?;

        let mut matches = Vec::new();
        for call in darkpool_calls.into_iter() {
            if let Some(match_result) = parse_external_match(&call.input)? {
                matches.push(match_result)
            }
        }

        Ok(matches)
    }
}
