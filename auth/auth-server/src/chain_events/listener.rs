//! Defines the core implementation of the on-chain event listener
//! Much of the implementation is borrowed from https://github.com/renegade-fi/renegade/blob/main/workers/chain-events/src/listener.rs

use std::{sync::Arc, thread::JoinHandle};

use alloy::{
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect},
    rpc::types::{Filter, trace::geth::CallFrame},
    sol_types::SolEvent,
};
use alloy_primitives::{Address, TxHash, U256};
use futures_util::StreamExt;
use price_reporter_client::PriceReporterClient;
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_common::types::chain::Chain;
use renegade_darkpool_client::DarkpoolClient;
use tracing::{error, info};

use crate::{
    bundle_store::BundleStore,
    chain_events::abis::{
        GasSponsorContract::{self},
        parse_external_match,
    },
};
use crate::{
    chain_events::abis::ExternalMatch,
    server::{
        gas_estimation::gas_cost_sampler::GasCostSampler, rate_limiter::AuthServerRateLimiter,
    },
};

use super::error::OnChainEventListenerError;

/// The nonce used event for the gas sponsor contract
type NonceUsed = GasSponsorContract::NonceUsed;

// ----------
// | Worker |
// ----------

/// The configuration passed to the listener upon startup
#[derive(Clone)]
pub struct OnChainEventListenerConfig {
    /// The chain for which the listener is configured
    pub(crate) chain: Chain,
    /// The address of the gas sponsor contract
    pub(crate) gas_sponsor_address: Address,
    /// The RPC websocket address to use for streaming events
    ///
    /// If not configured, the listener will poll using the darkpool client
    pub(crate) websocket_addr: Option<String>,
    /// The bundle store to use for retrieving bundle contexts
    pub(crate) bundle_store: BundleStore,
    /// The bundle rate limiter
    pub(crate) rate_limiter: AuthServerRateLimiter,
    /// The price reporter client with WebSocket streaming support
    pub(crate) price_reporter_client: PriceReporterClient,
    /// The gas cost sampler
    pub(crate) gas_cost_sampler: Arc<GasCostSampler>,
    /// A darkpool client for listening to events
    pub(crate) darkpool_client: DarkpoolClient,
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
    /// The chain for which the executor is configured
    pub(crate) chain: Chain,
    /// The address of the gas sponsor contract
    pub(crate) gas_sponsor_address: Address,
    /// The RPC websocket address to use for streaming events
    ///
    /// If not configured, the listener will poll using the darkpool client
    websocket_addr: Option<String>,
    /// The bundle store to use for retrieving bundle contexts
    bundle_store: BundleStore,
    /// The rate limiter
    pub(crate) rate_limiter: AuthServerRateLimiter,
    /// The price reporter client with WebSocket streaming support
    pub(crate) price_reporter_client: PriceReporterClient,
    /// The gas cost sampler
    pub(crate) gas_cost_sampler: Arc<GasCostSampler>,
    /// A darkpool client for listening to events
    pub(crate) darkpool_client: DarkpoolClient,
}

impl OnChainEventListenerExecutor {
    /// Create a new executor
    pub fn new(config: OnChainEventListenerConfig) -> Self {
        Self {
            chain: config.chain,
            gas_sponsor_address: config.gas_sponsor_address,
            websocket_addr: config.websocket_addr,
            bundle_store: config.bundle_store,
            rate_limiter: config.rate_limiter,
            price_reporter_client: config.price_reporter_client,
            gas_cost_sampler: config.gas_cost_sampler,
            darkpool_client: config.darkpool_client,
        }
    }

    // -----------
    // | Helpers |
    // -----------

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

    /// Get a provider to use for streaming logs
    pub async fn log_provider(&self) -> Result<DynProvider, OnChainEventListenerError> {
        let provider = if self.has_websocket_listener() {
            info!("Using websocket provider for log streaming");
            self.ws_client().await?
        } else {
            info!("Using HTTP provider for log streaming");
            self.darkpool_client.provider().clone()
        };

        Ok(provider)
    }

    // --------------
    // | Event Loop |
    // --------------

    /// The main execution loop for the executor
    pub async fn execute(self) -> Result<(), OnChainEventListenerError> {
        // Get the current block number to start from
        let starting_block_number = self
            .darkpool_client
            .block_number()
            .await
            .map_err(|err| OnChainEventListenerError::Darkpool(err.to_string()))?;
        info!("Starting on-chain event listener from current block {starting_block_number}");

        // Begin the watch loop
        let res = self.watch_nonces().await.unwrap_err();
        error!("on-chain event listener stream ended unexpectedly: {res}");
        Err(res)
    }

    /// Nonce watch loop
    async fn watch_nonces(&self) -> Result<(), OnChainEventListenerError> {
        // Build a log stream
        info!("listening for nonce used events");
        let provider = self.log_provider().await?;
        let filter = Filter::new().address(self.gas_sponsor_address).event(NonceUsed::SIGNATURE);
        let mut stream = provider.subscribe_logs(&filter).await?.into_stream();

        // Listen for events in a loop
        while let Some(log) = stream.next().await {
            let tx_hash = log
                .transaction_hash
                .ok_or_else(|| OnChainEventListenerError::darkpool("no tx hash"))?;

            let event = log.log_decode::<NonceUsed>()?;
            self.handle_nonce_used(tx_hash, event.data().nonce);
        }

        unreachable!()
    }

    // ------------------
    // | Nonce Handlers |
    // ------------------

    /// Handle a nonce used event
    fn handle_nonce_used(&self, tx: TxHash, nonce: U256) {
        let self_clone = self.clone();
        info!("handling nonce used event: {nonce}");
        tokio::spawn(async move {
            let res = self_clone.check_external_match_settlement(nonce, tx).await;
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
        nonce: U256,
        tx: TxHash,
    ) -> Result<(), OnChainEventListenerError> {
        let maybe_receipt = self
            .darkpool_client
            .provider()
            .get_transaction_receipt(tx)
            .await
            .map_err(OnChainEventListenerError::darkpool)?;

        let receipt = match maybe_receipt {
            Some(receipt) => receipt,
            None => {
                return Err(OnChainEventListenerError::darkpool("no receipt found for tx {tx:#x}"));
            },
        };

        // Get the time of settlement
        let settlement_time = self.get_settlement_timestamp(&receipt).await?;

        let matches = self.fetch_external_matches_in_tx(receipt.transaction_hash).await?;
        for external_match in matches {
            if let Some(bundle_ctx) = self.bundle_store.read(&nonce) {
                // Increase rate limit
                self.add_bundle_rate_limit_token(&bundle_ctx.key_description).await?;

                let api_match: ApiExternalMatchResult = external_match.match_result().into();

                // Record external match spread cost
                self.record_external_match_spread_cost(&bundle_ctx, &api_match).await?;

                // Record settlement metrics
                self.record_settlement_metrics(&receipt, &bundle_ctx, &api_match)?;

                // Record sponsorship metrics
                if let Some((gas_sponsorship_info, nonce)) = &bundle_ctx.gas_sponsorship_info {
                    self.record_settled_match_sponsorship(
                        &bundle_ctx,
                        &api_match,
                        &receipt,
                        gas_sponsorship_info,
                        *nonce,
                    )
                    .await?;
                }

                // Cleanup the bundle context
                self.bundle_store.remove_bundle(&bundle_ctx.bundle_id);

                // Record price sample to assembly delay
                self.record_assembly_delay(&bundle_ctx);

                // Record assembly to settlement delay
                self.record_assembly_to_settlement_delay(settlement_time, &bundle_ctx);

                // Record price sample to settlement delay
                self.record_settlement_delay(settlement_time, &bundle_ctx);
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
            self.darkpool_client.fetch_tx_darkpool_calls(tx).await?;

        let mut matches = Vec::new();
        for call in darkpool_calls.into_iter() {
            if let Some(match_result) = parse_external_match(&call.input)? {
                matches.push(match_result)
            }
        }

        Ok(matches)
    }
}
