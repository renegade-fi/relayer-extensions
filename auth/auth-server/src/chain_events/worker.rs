//! The worker implementation for the on-chain event listener
use std::thread::Builder;
use tokio::runtime::Builder as RuntimeBuilder;
use tracing::error;

use crate::store::BundleStore;

use super::{
    error::OnChainEventListenerError,
    listener::{OnChainEventListener, OnChainEventListenerConfig, OnChainEventListenerExecutor},
};

impl OnChainEventListener {
    pub fn new(
        config: OnChainEventListenerConfig,
        bundle_store: BundleStore,
    ) -> Result<Self, OnChainEventListenerError> {
        let executor = OnChainEventListenerExecutor::new(config, bundle_store);
        Ok(Self { executor: Some(executor), executor_handle: None })
    }

    pub fn start(&mut self) -> Result<(), OnChainEventListenerError> {
        // Spawn the execution loop in a separate thread
        let executor = self.executor.take().unwrap();
        let join_handle = Builder::new()
            .name("on-chain-event-listener-executor".to_string())
            .spawn(move || {
                let runtime = RuntimeBuilder::new_current_thread()
                    .enable_all()
                    .thread_name("on-chain-listener-runtime")
                    .build()
                    .map_err(|err| OnChainEventListenerError::Setup(err.to_string()));
                if let Err(e) = runtime {
                    return e;
                }

                let runtime = runtime.unwrap();
                runtime.block_on(async {
                    if let Err(e) = executor.execute().await {
                        error!("Chain event listener crashed with error: {e}");
                        return e;
                    }

                    OnChainEventListenerError::StreamEnded
                })
            })
            .map_err(|err| OnChainEventListenerError::Setup(err.to_string()))?;

        self.executor_handle = Some(join_handle);
        Ok(())
    }

    /// Spawns a watcher thread that joins the given handle and logs its
    /// outcome.
    pub fn watch(mut self) {
        std::thread::Builder::new()
            .name("on-chain-listener-watcher".to_string())
            .spawn(move || match self.executor_handle.take().unwrap().join() {
                Err(panic) => {
                    error!("worker on-chain-event-listener panicked with error: {panic:?}");
                },
                Ok(err) => {
                    error!("worker on-chain-event-listener exited with error: {err:?}");
                },
            })
            .unwrap();
    }
}
