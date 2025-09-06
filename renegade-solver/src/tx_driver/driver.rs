//! Defines the transaction driver which schedules submissions by timestamp
//! millis.

use alloy_primitives::Bytes;
use eyre::Result;
use renegade_util::get_current_time_millis;
use tokio::time::{sleep_until, Duration, Instant};

use crate::{
    arrival_control::controller::ArrivalController,
    tx_store::store::{OrderHash, TxStore},
    uniswapx::executor_client::ExecutorClient,
};

/// The driver for the transaction scheduler.
#[derive(Clone)]
pub struct TxDriver {
    /// Arrival controller
    arrival_controller: ArrivalController,
    /// Executor client used for sending transactions
    executor_client: ExecutorClient,
    /// Tx store
    tx_store: TxStore,
}

impl TxDriver {
    /// Creates a new `TxDriver` with the given executor
    /// client.
    pub fn new(
        arrival_controller: &ArrivalController,
        executor_client: &ExecutorClient,
        tx_store: &TxStore,
    ) -> Self {
        Self {
            arrival_controller: arrival_controller.clone(),
            executor_client: executor_client.clone(),
            tx_store: tx_store.clone(),
        }
    }

    /// Send a pre-computed transaction at the specified wall-clock instant.
    async fn send_tx(
        id: OrderHash,
        raw_tx_bytes: Bytes,
        send_ts: u64,
        executor_client: ExecutorClient,
        arrival_controller: ArrivalController,
        tx_store: TxStore,
    ) -> Result<()> {
        // Sleep until the send time
        let now = get_current_time_millis();
        let delay_ms = send_ts.saturating_sub(now);
        let deadline = Instant::now() + Duration::from_millis(delay_ms);
        sleep_until(deadline).await;

        // Submit transaction
        let submitted_ts = get_current_time_millis();
        let tx_hash = executor_client.send_raw(raw_tx_bytes).await?;

        // Record submission
        let actual_ts = get_current_time_millis();
        arrival_controller.on_feedback(submitted_ts, actual_ts);
        tx_store.record_submission(&id, submitted_ts);

        // Log
        tracing::info!("actual - submitted: {}ms", actual_ts - submitted_ts);
        tracing::info!(message = "shot out", tx_hash = %tx_hash);
        Ok(())
    }

    /// Queue a transaction to be sent at the given timestamp milliseconds.
    pub fn enqueue(&self, id: OrderHash, raw_tx_bytes: &Bytes, send_ts: u64) {
        let raw_tx_bytes = raw_tx_bytes.clone();
        let arrival_controller = self.arrival_controller.clone();
        let executor_client = self.executor_client.clone();
        let tx_store = self.tx_store.clone();

        tokio::spawn(async move {
            if let Err(err) = Self::send_tx(
                id,
                raw_tx_bytes,
                send_ts,
                executor_client,
                arrival_controller,
                tx_store,
            )
            .await
            {
                tracing::warn!("Tx submission failed with error: {}", err);
            }
        });
    }
}
