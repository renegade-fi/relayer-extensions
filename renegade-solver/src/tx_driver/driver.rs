//! Defines the transaction driver which schedules submissions by timestamp
//! millis.

use alloy_primitives::{Bytes, TxHash};
use eyre::Result;
use tokio::time::{sleep_until, Duration, Instant};

use crate::{
    flashblocks::clock::get_current_time_millis, uniswapx::executor_client::ExecutorClient,
};

/// The driver for the transaction scheduler.
#[derive(Clone)]
pub struct TxDriver {
    /// Executor client used for sending transactions
    executor_client: ExecutorClient,
}

impl TxDriver {
    /// Creates a new `TxDriver` with the given executor
    /// client.
    pub fn new(executor: &ExecutorClient) -> Self {
        Self { executor_client: executor.clone() }
    }

    /// Send a pre-computed transaction at the specified wall-clock instant.
    async fn send_tx(
        send_at: Instant,
        raw_tx_bytes: Bytes,
        tx_hash: TxHash,
        executor_client: ExecutorClient,
    ) -> Result<()> {
        sleep_until(send_at).await;
        executor_client.send_raw(raw_tx_bytes).await?;
        tracing::info!(message = "shot out", tx_hash = %tx_hash);
        Ok(())
    }

    /// Queue a transaction to be sent at the given timestamp milliseconds.
    pub fn enqueue(&self, send_timestamp_ms: u64, raw_tx_bytes: &Bytes, tx_hash: &TxHash) {
        let now_ms = get_current_time_millis();
        let delay_ms = send_timestamp_ms.saturating_sub(now_ms);
        let send_at = Instant::now() + Duration::from_millis(delay_ms);

        let executor_client = self.executor_client.clone();
        let raw_tx_bytes = raw_tx_bytes.clone();
        let tx_hash = *tx_hash;

        tokio::spawn(async move {
            if let Err(err) = Self::send_tx(send_at, raw_tx_bytes, tx_hash, executor_client).await {
                tracing::warn!("Tx submission failed with error: {}", err);
            }
        });
    }
}
