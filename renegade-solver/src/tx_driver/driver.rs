//! Defines the transaction driver which is responsible for scheduling
//! transactions to be submitted on-chain.

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep_until, Instant};

use crate::flashblocks::{Flashblock, FlashblocksReceiver};
use crate::tx_store::store::{L2Position, TxStore};
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
        let (tx, rx) = unbounded_channel();
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
            let tx_store_clone = tx_store.clone();
            let executor_client_clone = executor_client.clone();
            tokio::spawn(Self::handle_scheduled_tx(
                tx_id,
                at,
                tx_store_clone,
                executor_client_clone,
            ));
        }
    }

    /// Handles a single scheduled transaction: waits until the target time and
    /// submits it.
    async fn handle_scheduled_tx(
        tx_id: String,
        at: Instant,
        tx_store: TxStore,
        executor_client: ExecutorClient,
    ) {
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
                        tracing::warn!(id = %tx_id, err = %err, "error sending tx");
                    },
                }
            },
            Err(err) => {
                tracing::warn!(id = %tx_id, err = %err, "failed to hydrate and send tx");
            },
        }
    }
}

impl FlashblocksReceiver for TxDriver {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let position = L2Position { l2_block: fb.metadata.block_number, flashblock: fb.index };
        let ready_txns = self.tx_store.due_at(&position, fb.received_at);
        for (id, send_at) in ready_txns {
            let _ = self.scheduler.send((id, send_at.into()));
        }
    }
}
