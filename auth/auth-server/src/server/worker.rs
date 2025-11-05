//! The worker implementation for the HTTP server

use std::{
    net::SocketAddr,
    sync::Arc,
    thread::{Builder, JoinHandle},
};

use tokio::runtime::Builder as RuntimeBuilder;
use tracing::error;

use super::{Server, http::HttpServerExecutor};

/// The error type for HTTP server worker
pub type HttpServerError = String;

/// The configuration for the HTTP server worker
pub struct HttpServerConfig {
    /// The server instance
    pub server: Arc<Server>,
    /// The address to bind the server to
    pub listen_addr: SocketAddr,
}

/// The HTTP server worker
pub struct HttpServerWorker {
    /// The executor run in a separate thread
    executor: Option<HttpServerExecutor>,
    /// The thread handle of the executor
    executor_handle: Option<JoinHandle<HttpServerError>>,
}

impl HttpServerWorker {
    /// Create a new HTTP server worker
    pub fn new(config: HttpServerConfig) -> Result<Self, HttpServerError> {
        let executor = HttpServerExecutor::new(config.server, config.listen_addr);
        Ok(Self { executor: Some(executor), executor_handle: None })
    }

    /// Start the HTTP server on its own runtime
    pub fn start(&mut self) -> Result<(), HttpServerError> {
        let executor = self.executor.take().unwrap();
        let join_handle = Builder::new()
            .name("http-server-executor".to_string())
            .spawn(move || {
                let runtime = RuntimeBuilder::new_current_thread()
                    .enable_all()
                    .thread_name("http-server-runtime")
                    .build()
                    .map_err(|err| format!("Failed to build runtime: {err}"));
                if let Err(e) = runtime {
                    return e;
                }

                let runtime = runtime.unwrap();
                runtime.block_on(async {
                    executor.execute().await;
                    "HTTP server stopped unexpectedly".to_string()
                })
            })
            .map_err(|err| format!("Failed to spawn thread: {err}"))?;

        self.executor_handle = Some(join_handle);
        Ok(())
    }

    /// Returns a name by which the worker can be identified
    pub fn name(&self) -> String {
        "http-server".to_string()
    }

    /// Spawns a watcher thread that joins the given handle and logs its
    /// outcome, sending a failure signal if the worker exits.
    pub fn watch(&mut self, failure_tx: &tokio::sync::mpsc::Sender<()>) {
        let worker_name = self.name();
        let join_handle = self.executor_handle.take().unwrap();
        let failure_tx_clone = failure_tx.clone();

        Builder::new()
            .name(format!("{worker_name}-watcher"))
            .spawn(move || {
                match join_handle.join() {
                    Err(panic) => {
                        error!("worker {worker_name} panicked with error: {panic:?}");
                    },
                    Ok(err) => {
                        error!("worker {worker_name} exited with error: {err:?}");
                    },
                }
                failure_tx_clone.blocking_send(()).unwrap();
            })
            .unwrap();
    }
}
