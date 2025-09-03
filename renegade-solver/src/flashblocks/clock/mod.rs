//! Defines the FlashblockClock which is capable of estimating the time at which
//! we will observe a flashblock. To do this, it uses an EMA to track the
//! frequency at which we observe flashblocks and L2 blocks over the WebSocket
//! connection to the Flashblocks block builder.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::flashblocks::{Flashblock, FlashblocksReceiver};

use self::ema::Ema;

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
    ema: Ema,
}

impl FlashblockClockInner {
    /// Creates a new FlashblockClockInner.
    fn new() -> Self {
        Self {
            last_flashblock_idx: AtomicU64::new(0),
            last_l2_idx: AtomicU64::new(0),
            last_flashblock_ts: AtomicU64::new(0),
            last_l2_ts: AtomicU64::new(0),
            ema: Ema::new(),
        }
    }
}

impl FlashblockClock {
    /// Creates a new FlashblockClock.
    pub fn new() -> Self {
        Self(Arc::new(FlashblockClockInner::new()))
    }

    /// Returns the estimated target timestamp in milliseconds for the given
    /// target flashblock and L2 block.
    pub fn target_timestamp_ms(&self, target_flashblock: u64, target_l2: u64) -> u64 {
        // Snapshot estimates/anchors.
        let FlashblockClockSnapshot {
            flashblock_duration_ms,
            l2_block_duration_ms,
            last_l2_idx,
            last_l2_ts,
            last_flashblock_idx,
            last_flashblock_ts,
        } = self.snapshot();

        // If we're in the same L2 block as the last observed flashblock, use the last
        // flashblock timestamp.
        if target_l2 == last_l2_idx && last_flashblock_ts != 0 {
            let delta_flashblock = target_flashblock.saturating_sub(last_flashblock_idx);
            return last_flashblock_ts + delta_flashblock.saturating_mul(flashblock_duration_ms);
        }

        // If we're in a different L2 block, use the last observed L2 block timestamp.
        if last_l2_ts != 0 {
            let delta_l2 = target_l2.saturating_sub(last_l2_idx);
            return last_l2_ts
                + delta_l2.saturating_mul(l2_block_duration_ms)
                + target_flashblock.saturating_mul(flashblock_duration_ms);
        }

        // If we don't have any estimates, use the current time.
        let now = get_current_time_millis();
        now + target_l2.saturating_mul(l2_block_duration_ms)
            + target_flashblock.saturating_mul(flashblock_duration_ms)
    }

    /// Update the state from the observation of a flashblock and L2 block.
    pub fn update_from_observation(&self, current_fb_idx: u64, current_l2_idx: u64, now: u64) {
        self.0.last_flashblock_idx.store(current_fb_idx, Ordering::Relaxed);
        self.0.last_flashblock_ts.store(now, Ordering::Relaxed);
        let last_l2_idx = self.0.last_l2_idx.load(Ordering::Relaxed);
        if current_l2_idx != last_l2_idx {
            self.0.last_l2_idx.store(current_l2_idx, Ordering::Relaxed);
            self.0.last_l2_ts.store(now, Ordering::Relaxed);
        }
    }

    /// Snapshot the current state of the clock.
    pub fn snapshot(&self) -> FlashblockClockSnapshot {
        FlashblockClockSnapshot {
            flashblock_duration_ms: self.0.ema.flashblock_duration_ms(),
            l2_block_duration_ms: self.0.ema.l2_block_duration_ms(),
            last_l2_idx: self.0.last_l2_idx.load(Ordering::Relaxed),
            last_l2_ts: self.0.last_l2_ts.load(Ordering::Relaxed),
            last_flashblock_idx: self.0.last_flashblock_idx.load(Ordering::Relaxed),
            last_flashblock_ts: self.0.last_flashblock_ts.load(Ordering::Relaxed),
        }
    }
}

/// Returns the current unix timestamp in milliseconds, represented as u64.
pub fn get_current_time_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).expect("negative timestamp").as_millis() as u64
}

impl FlashblocksReceiver for FlashblockClock {
    fn on_flashblock_received(&self, fb: Flashblock) {
        let now = get_current_time_millis();
        let current_fb_idx = fb.index;
        let current_l2_idx = fb.metadata.block_number;

        // Update EMA
        let FlashblockClockSnapshot {
            last_l2_idx,
            last_l2_ts,
            last_flashblock_idx,
            last_flashblock_ts,
            flashblock_duration_ms: _,
            l2_block_duration_ms: _,
        } = self.snapshot();
        self.0.ema.update_estimates(
            last_flashblock_idx,
            last_flashblock_ts,
            last_l2_idx,
            last_l2_ts,
            current_fb_idx,
            current_l2_idx,
            now,
        );

        // Update the state of the flashblock clock.
        self.update_from_observation(current_fb_idx, current_l2_idx, now);
    }
}
