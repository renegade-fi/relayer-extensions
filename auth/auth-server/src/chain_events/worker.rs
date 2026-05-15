//! The worker implementation for the on-chain event listener
use std::thread::Builder;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio_util::sync::CancellationToken;

use super::{
    error::OnChainEventListenerError,
    listener::{OnChainEventListener, OnChainEventListenerConfig, OnChainEventListenerExecutor},
};
use crate::log_task;
use crate::logger::{Outcome, Task};

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
                        log_task!(
                            Task::ChainEventListener,
                            Outcome::Failed,
                            subject = "executor",
                            error = %e,
                            "chain event listener crashed"
                        );
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
    pub fn watch(mut self, cancellation_token: CancellationToken) {
        std::thread::Builder::new()
            .name("on-chain-listener-watcher".to_string())
            .spawn(move || {
                match self.executor_handle.take().unwrap().join() {
                    Err(panic) => {
                        log_task!(
                            Task::ChainEventListener,
                            Outcome::Failed,
                            subject = "worker-panic",
                            panic = ?panic,
                            "worker thread panicked"
                        );
                    },
                    Ok(err) => {
                        log_task!(
                            Task::ChainEventListener,
                            Outcome::Failed,
                            subject = "worker-exit",
                            error = ?err,
                            "worker thread exited with error"
                        );
                    },
                }
                cancellation_token.cancel();
            })
            .unwrap();
    }
}
