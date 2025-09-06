//! Defines the FlashblockClock which is capable of estimating the time at which
//! we will observe a flashblock. To do this, it uses an EMA to track the
//! frequency at which we observe flashblocks and L2 blocks over the WebSocket
//! connection to the Flashblocks block builder.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use alloy_primitives::BlockNumber;
use renegade_util::get_current_time_millis;

use crate::flashblocks::clock::constants::{
    BLOCK_BUILDER_DEADLINE_LEAD_TIME_MS, FIRST_FLASHBLOCK_LEAD_TIME_MS,
};
use crate::flashblocks::{Flashblock, FlashblocksReceiver};

use self::ema::EmaManager;

mod constants;
mod ema;

/// A snapshot of the current state of the FlashblockClock.
pub struct FlashblockClockSnapshot {
    /// The estimated L2 block duration in milliseconds.
    pub l2_block_duration_ms: u64,
    /// The estimated flashblock duration in milliseconds.
    pub flashblock_duration_ms: u64,
    /// The last observed L2 block index.
    pub last_l2_idx: u64,
    /// The last observed L2 block timestamp in milliseconds.
    pub last_l2_ts: u64,
    /// The last observed flashblock index.
    pub last_flashblock_idx: u64,
    /// The last observed flashblock timestamp in milliseconds.
    pub last_flashblock_ts: u64,
    /// The current EMA estimate of the websocket forecast error.
    pub forecast_error_ms: f64,
}

/// A thread-safe clock that can estimate the time at which we will observe a
/// flashblock.
#[derive(Clone)]
pub struct FlashblockClock(pub Arc<FlashblockClockInner>);

/// The inner state of the FlashblockClock.
pub struct FlashblockClockInner {
    /// The last observed flashblock index
    pub last_flashblock_idx: AtomicU64,
    /// The last observed L2 block
    pub last_l2_idx: AtomicU64,
    /// The last observed flashblock timestamp in milliseconds
    pub last_flashblock_ts: AtomicU64,
    /// The last observed L2 block timestamp in milliseconds
    pub last_l2_ts: AtomicU64,
    /// The EMA of the flashblock and L2 block durations.
    ema: EmaManager,
}

impl FlashblockClockInner {
    /// Creates a new FlashblockClockInner.
    fn new() -> Self {
        Self {
            last_flashblock_idx: AtomicU64::new(0),
            last_l2_idx: AtomicU64::new(0),
            last_flashblock_ts: AtomicU64::new(0),
            last_l2_ts: AtomicU64::new(0),
            ema: EmaManager::new(),
        }
    }
}

impl FlashblockClock {
    /// Creates a new FlashblockClock.
    pub fn new() -> Self {
        Self(Arc::new(FlashblockClockInner::new()))
    }

    /// Given a flashblock and block number we use
    /// - the last observed L2 block
    /// - the average flash(block) durations
    ///
    /// to predict the timestamp at which we will observe the flashblock over
    /// the Websocket.
    ///
    /// Returns the estimated target timestamp in milliseconds since the Unix
    /// epoch.
    pub fn predict_flashblock_ts(&self, flashblock: u64, block: BlockNumber) -> Option<u64> {
        let FlashblockClockSnapshot {
            flashblock_duration_ms,
            l2_block_duration_ms,
            last_l2_idx,
            last_l2_ts,
            ..
        } = self.snapshot();

        // If we don't have any observations, return 0.
        if last_l2_ts == 0 {
            return None;
        }

        // Always use the last observed L2 block as our anchor point
        let l2_blocks_delta = block.saturating_sub(last_l2_idx);
        let l2_block_offset = last_l2_ts + l2_blocks_delta.saturating_mul(l2_block_duration_ms);

        // Add the flashblock offset
        let flashblock_offset = flashblock.saturating_mul(flashblock_duration_ms);

        Some(l2_block_offset + flashblock_offset)
    }

    /// Given a flashblock and block number we use
    /// - the average flash(block) durations
    /// - the learned forecast error
    /// - the builder deadline lead time
    /// - the first flashblock lead time
    ///
    /// to predict the timestamp at which a transaction should arrive in the
    /// block builder's inbox.
    ///
    /// Returns this timestamp in milliseconds since the Unix epoch.
    pub fn predict_adjusted_flashblock_ts(
        &self,
        flashblock: u64,
        block: BlockNumber,
    ) -> Option<u64> {
        if let Some(predicted_ts) = self.predict_flashblock_ts(flashblock, block) {
            let forecast_error_ms = self.forecast_error_ms();
            let adjusted_ts = predicted_ts
                .saturating_sub(forecast_error_ms.abs() as u64)
                .saturating_sub(BLOCK_BUILDER_DEADLINE_LEAD_TIME_MS)
                .saturating_sub(FIRST_FLASHBLOCK_LEAD_TIME_MS);

            Some(adjusted_ts)
        } else {
            None
        }
    }

    /// Returns the current EMA estimate of the websocket forecast error.
    ///
    /// Websocket forecast error is the difference between the actual timestamp
    /// and the predicted timestamp.
    fn forecast_error_ms(&self) -> f64 {
        self.0.ema.forecast_error_ms()
    }

    /// Update the state from the observation of a flashblock and L2 block.
    pub fn update_from_observation(&self, current_fb_idx: u64, current_l2_idx: u64, now: u64) {
        // Update the last observed flashblock index and timestamp
        self.0.last_flashblock_idx.store(current_fb_idx, Ordering::Relaxed);
        self.0.last_flashblock_ts.store(now, Ordering::Relaxed);

        // Update the last observed L2 block index and timestamp
        let last_l2_idx = self.0.last_l2_idx.load(Ordering::Relaxed);
        if current_l2_idx != last_l2_idx {
            self.0.last_l2_idx.store(current_l2_idx, Ordering::Relaxed);
            self.0.last_l2_ts.store(now, Ordering::Relaxed);
        }
    }

    /// Snapshot the current state of the clock.
    pub fn snapshot(&self) -> FlashblockClockSnapshot {
        let inner = &self.0;
        let ema = &inner.ema;
        FlashblockClockSnapshot {
            flashblock_duration_ms: ema.flashblock_duration_ms(),
            l2_block_duration_ms: ema.l2_block_duration_ms(),
            last_l2_idx: inner.last_l2_idx.load(Ordering::Relaxed),
            last_l2_ts: inner.last_l2_ts.load(Ordering::Relaxed),
            last_flashblock_idx: inner.last_flashblock_idx.load(Ordering::Relaxed),
            last_flashblock_ts: inner.last_flashblock_ts.load(Ordering::Relaxed),
            forecast_error_ms: ema.forecast_error_ms(),
        }
    }
}

impl FlashblocksReceiver for FlashblockClock {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let current_fb_idx = fb.index;
        let current_l2_idx = fb.metadata.block_number;

        let FlashblockClockSnapshot {
            last_l2_idx,
            last_l2_ts,
            last_flashblock_idx,
            last_flashblock_ts,
            ..
        } = self.snapshot();

        // Update EMA
        let ema = &self.0.ema;
        let now_ms = get_current_time_millis();
        // Only update if we have valid samples
        if let Some(fb_sample) = ema.maybe_sample_flashblock_duration(
            last_flashblock_idx,
            last_flashblock_ts,
            current_fb_idx,
            now_ms,
        ) {
            ema.update_fb_estimate(fb_sample);
        }

        if let Some(l2_sample) =
            ema.maybe_sample_l2_duration(last_l2_idx, last_l2_ts, current_l2_idx, now_ms)
        {
            ema.update_l2_estimate(l2_sample);
        }

        if let Some(predicted_ts) = self.predict_flashblock_ts(current_fb_idx, current_l2_idx) {
            ema.update_forecast_error_estimate(now_ms as f64, predicted_ts as f64);
        }

        // Update the state of the flashblock clock.
        self.update_from_observation(current_fb_idx, current_l2_idx, now_ms);
    }
}

impl Default for FlashblockClock {
    fn default() -> Self {
        Self::new()
    }
}
