//! Defines a worker that listens for blocks and updates the chain state cache.

use alloy::{
    providers::{DynProvider, Provider},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::Address;
use futures_util::StreamExt;
use std::str::FromStr;
use tracing::{error, info, warn};

use crate::{chain_state_cache::cache::ChainStateCache, cli::Cli};

/// The worker that listens for blocks and updates the chain state cache.
pub struct ChainStateCacheWorker {
    /// The provider to use to subscribe to blocks.
    provider: DynProvider,
    /// The chain state cache to update.
    chain_state_cache: ChainStateCache,
    /// Signer whose pending nonce is tracked
    signer_address: Address,
}

impl ChainStateCacheWorker {
    /// Creates a new `FeeCacheWorker` with the given provider and chain state
    /// cache.
    pub fn new(provider: DynProvider, chain_state_cache: ChainStateCache, cli: &Cli) -> Self {
        let signer = PrivateKeySigner::from_str(&cli.private_key).expect("Failed to parse signer");
        let signer_address = signer.address();
        Self { provider, chain_state_cache, signer_address }
    }

    /// Starts the worker.
    pub fn start(&self) {
        let provider = self.provider.clone();
        let chain_state_cache = self.chain_state_cache.clone();
        let signer = self.signer_address;
        tokio::spawn(Self::watch_blocks(provider, chain_state_cache, signer));
    }

    /// Watch for blocks and update the chain state cache via a websocket
    /// stream.
    async fn watch_blocks(
        provider: DynProvider,
        chain_state_cache: ChainStateCache,
        signer: Address,
    ) {
        match provider.subscribe_blocks().await {
            Ok(subscription) => {
                info!("listening for blocks via websocket");
                let mut stream = subscription.into_stream();
                while let Some(header) = stream.next().await {
                    if let Some(base) = header.base_fee_per_gas {
                        chain_state_cache.set_base_fee_per_gas(base);
                    }
                    // Update pending nonce on each block tick
                    match provider.get_transaction_count(signer).await {
                        Ok(nonce) => chain_state_cache.set_pending_nonce(nonce),
                        Err(e) => warn!("failed to refresh pending nonce: {}", e),
                    }
                }
                warn!("block stream ended");
            },
            Err(e) => error!("subscription error: {e}"),
        }
    }
}
