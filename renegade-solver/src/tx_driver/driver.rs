//! Defines the transaction driver which is responsible for scheduling
//! transactions to be submitted on-chain.

use alloy_primitives::{Bytes, TxHash};
use eyre::Result;
use tokio::time::{sleep_until, Duration, Instant};

use crate::flashblocks::{Flashblock, FlashblocksReceiver};
use crate::tx_store::store::{L2Position, TxStore};
use crate::uniswapx::executor_client::ExecutorClient;

/// The driver for the transaction scheduler.
#[derive(Clone)]
pub struct TxDriver {
    /// The transaction store
    tx_store: TxStore,
    /// Executor client used for sending transactions
    executor_client: ExecutorClient,
}

impl TxDriver {
    /// Creates a new `TxDriver` with the given transaction store and executor
    /// client.
    pub fn new(tx_store: TxStore, executor: &ExecutorClient) -> Self {
        Self { tx_store, executor_client: executor.clone() }
    }

    /// Send a pre-computed transaction at the specified time
    async fn send_tx(
        id: String,
        send_at: Instant,
        raw_tx_bytes: Bytes,
        tx_hash: TxHash,
        executor_client: ExecutorClient,
    ) -> Result<()> {
        // Wait until the send time
        sleep_until(send_at).await;

        // Send the pre-signed transaction
        executor_client.send_raw(raw_tx_bytes).await?;

        tracing::info!(message = "shot out", id = %id, tx_hash = %tx_hash);
        Ok(())
    }
}

impl FlashblocksReceiver for TxDriver {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let position = L2Position { l2_block: fb.metadata.block_number, flashblock: fb.index };
        let ready_txns = self.tx_store.due_at(&position);

        for (id, buffer_ms, raw_tx_bytes, tx_hash) in ready_txns {
            let executor_client = self.executor_client.clone();
            let buffer_duration = Duration::from_millis(buffer_ms);
            let send_at = fb.received_at.checked_add(buffer_duration).unwrap();

            tokio::spawn(async move {
                if let Err(err) = Self::send_tx(
                    id.clone(),
                    send_at.into(),
                    raw_tx_bytes,
                    tx_hash,
                    executor_client,
                )
                .await
                {
                    tracing::warn!(message = "send_tx failed", id = %id, err = %err);
                }
            });
        }
    }
}
