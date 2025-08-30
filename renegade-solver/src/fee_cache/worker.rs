//! Defines a worker that listens for blocks and updates the fee cache.

use alloy::{
    providers::{DynProvider, Provider},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::Address;
use futures_util::StreamExt;
use std::str::FromStr;
use tracing::{error, info, warn};

use crate::{cli::Cli, fee_cache::fees::FeeCache};

/// The worker that listens for blocks and updates the fee cache.
pub struct FeeCacheWorker {
    /// The provider to use to subscribe to blocks.
    provider: DynProvider,
    /// The fee cache to update.
    fee_cache: FeeCache,
    /// Signer address whose pending nonce is tracked
    signer_address: Address,
}

impl FeeCacheWorker {
    /// Creates a new `FeeCacheWorker` with the given provider and fee cache.
    pub fn new(provider: DynProvider, fee_cache: FeeCache, cli: &Cli) -> Self {
        let private_key =
            PrivateKeySigner::from_str(&cli.private_key).expect("Failed to parse private key");
        let signer_address = private_key.address();
        Self { provider, fee_cache, signer_address }
    }

    /// Starts the worker.
    pub fn start(&self) {
        let provider = self.provider.clone();
        let fee_cache = self.fee_cache.clone();
        let signer = self.signer_address;
        tokio::spawn(Self::watch_blocks(provider, fee_cache, signer));
    }

    /// Watch for blocks and update the fee cache via a websocket stream.
    async fn watch_blocks(provider: DynProvider, fee_cache: FeeCache, signer: Address) {
        match provider.subscribe_blocks().await {
            Ok(subscription) => {
                info!("listening for blocks via websocket");
                let mut stream = subscription.into_stream();
                while let Some(header) = stream.next().await {
                    if let Some(base) = header.base_fee_per_gas {
                        fee_cache.set_base_fee_per_gas(base);
                    }
                    // Update pending nonce on each block tick
                    match provider.get_transaction_count(signer).await {
                        Ok(nonce) => fee_cache.set_pending_nonce(nonce),
                        Err(e) => warn!("failed to refresh pending nonce: {}", e),
                    }
                }
                warn!("block stream ended");
            },
            Err(e) => error!("subscription error: {e}"),
        }
    }
}
