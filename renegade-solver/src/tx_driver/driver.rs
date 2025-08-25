//! Defines the transaction driver which is responsible for scheduling
//! transactions to be submitted on-chain.

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Instant};

use crate::flashblocks::{Flashblock, FlashblocksReceiver};
use crate::tx_store::store::TxStore;
use crate::uniswapx::executor_client::ExecutorClient;

/// The driver for the transaction scheduler.
#[derive(Clone)]
pub struct TxDriver {
    /// The sender for the transaction scheduler.
    scheduler: UnboundedSender<(String, Instant)>,
    /// The transaction store
    tx_store: TxStore,
}

impl TxDriver {
    /// Creates a new `TxDriver` with the given transaction store and executor
    /// client.
    pub fn new(tx_store: TxStore, executor: &ExecutorClient) -> Self {
        let (tx, mut rx) = unbounded_channel::<(String, Instant)>();
        let tx_store_clone = tx_store.clone();
        let executor_client_clone = executor.clone();

        tokio::spawn(Self::run_scheduler(rx, tx_store_clone, executor_client_clone));

        Self { tx_store, scheduler: tx }
    }

    /// Background task: drains scheduled items and submits transactions at
    /// their target times.
    async fn run_scheduler(
        mut rx: UnboundedReceiver<(String, Instant)>,
        tx_store: TxStore,
        executor_client: ExecutorClient,
    ) {
        while let Some((tx_id, at)) = rx.recv().await {
            sleep_until(at).await;

            match tx_store.resolve_fee_caps(&tx_id) {
                Ok(tx) => {
                    tracing::info!(id = %tx_id, "taking the shot");
                    match executor_client.send_tx(tx).await {
                        Ok(tx_hash) => {
                            tx_store.record_tx_hash(&tx_id, tx_hash);
                            tracing::info!(id = %tx_id, tx_hash = %tx_hash, "shot out");
                        },
                        Err(err) => {
                            tracing::warn!(%err, id = %tx_id, "error sending tx");
                        },
                    }
                },
                Err(err) => {
                    tracing::warn!(%err, id = %tx_id, "unable to hydrate tx with base fee");
                },
            }
        }
    }
}

impl FlashblocksReceiver for TxDriver {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let ready_txns = self.tx_store.due_at(fb.metadata.block_number, fb.index, fb.received_at);
        for (id, send_at) in ready_txns {
            let _ = self.scheduler.send((id, send_at.into()));
        }
    }
}
