//! The worker implementation for the on-chain event listener
use std::thread::Builder;
use tokio::runtime::Builder as RuntimeBuilder;
use tracing::error;

use super::{
    error::OnChainEventListenerError,
    listener::{OnChainEventListener, OnChainEventListenerConfig, OnChainEventListenerExecutor},
};

impl OnChainEventListener {
    /// Create a new on-chain event listener
    pub fn new(config: OnChainEventListenerConfig) -> Result<Self, OnChainEventListenerError> {
        let executor = OnChainEventListenerExecutor::new(config);
        Ok(Self { executor: Some(executor), executor_handle: None })
    }

    /// Start the listener on its own runtime
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

    /// Spawns a watcher thread that joins the given handle and panics if the
    /// executor thread panics or exits with an error, ensuring failures crash
    /// the process.
    pub fn watch(mut self) {
        std::thread::Builder::new()
            .name("on-chain-listener-watcher".to_string())
            .spawn(move || match self.executor_handle.take().unwrap().join() {
                Err(panic) => {
                    panic!("worker on-chain-event-listener panicked: {panic:?}");
                },
                Ok(err) => {
                    panic!("worker on-chain-event-listener exited with error: {err:?}");
                },
            })
            .unwrap();
    }
}
