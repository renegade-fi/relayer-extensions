//! Defines a worker that listens for blocks and updates the fee cache.

use alloy::providers::{DynProvider, Provider};
use futures_util::StreamExt;
use tracing::{error, info, warn};

use crate::fee_cache::fees::FeeCache;

/// The worker that listens for blocks and updates the fee cache.
pub struct FeeCacheWorker {
    /// The provider to use to subscribe to blocks.
    provider: DynProvider,
    /// The fee cache to update.
    fee_cache: FeeCache,
}

impl FeeCacheWorker {
    /// Creates a new `FeeCacheWorker` with the given provider and fee cache.
    pub fn new(provider: DynProvider, fee_cache: FeeCache) -> Self {
        Self { provider, fee_cache }
    }

    /// Starts the worker.
    pub fn start(&self) {
        let provider = self.provider.clone();
        let fee_cache = self.fee_cache.clone();
        tokio::spawn(async move {
            match provider.subscribe_blocks().await {
                Ok(subscription) => {
                    info!("listening for blocks via websocket");
                    let mut stream = subscription.into_stream();
                    while let Some(header) = stream.next().await {
                        if let Some(base) = header.base_fee_per_gas {
                            fee_cache.set_base_fee_per_gas(base);
                        }
                    }
                    warn!("block stream ended");
                },
                Err(e) => error!("subscription error: {e}"),
            }
        });
    }
}
