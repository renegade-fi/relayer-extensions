//! EMA manager that updates estimates once aligned to clean boundaries.
//! The clock module only needs to call `update()` and get estimates.

use std::sync::atomic::Ordering;

use atomic_float::AtomicF64;

use super::constants::{DEFAULT_FLASHBLOCK_MS, DEFAULT_L2_MS, FLASHBLOCK_WINDOW, L2_WINDOW};

/// EMA manager for flashblock and L2 timing.
///
/// Maintains two independent EMA estimates for flashblock and L2 block
/// durations.
pub(crate) struct Ema {
    /// The smoothing factor for the flashblock duration.
    fb_alpha: f64,
    /// The smoothing factor for the L2 block duration.
    l2_alpha: f64,
    /// Current EMA estiamte of the flashblock duration.
    fb_estimate_f64: AtomicF64,
    /// Current EMA estiamte of the L2 block duration.
    l2_estimate_f64: AtomicF64,
}

impl Ema {
    /// Creates a new EMA manager.
    pub fn new() -> Self {
        Self {
            fb_alpha: alpha_from_window(FLASHBLOCK_WINDOW),
            l2_alpha: alpha_from_window(L2_WINDOW),
            fb_estimate_f64: AtomicF64::new(DEFAULT_FLASHBLOCK_MS as f64),
            l2_estimate_f64: AtomicF64::new(DEFAULT_L2_MS as f64),
        }
    }

    /// Returns the current EMA estimate of the flashblock duration.
    pub fn flashblock_duration_ms(&self) -> u64 {
        let est = self.fb_estimate_f64.load(Ordering::Relaxed);
        est.round() as u64
    }

    /// Returns the current EMA estimate of the L2 block duration.
    pub fn l2_block_duration_ms(&self) -> u64 {
        let est = self.l2_estimate_f64.load(Ordering::Relaxed);
        est.round() as u64
    }

    #[allow(clippy::too_many_arguments)]
    /// Updates the EMA estimates of the flashblock and L2 block durations.
    pub fn update_estimates(
        &self,
        last_flashblock_idx: u64,
        last_flashblock_ts: u64,
        last_l2_idx: u64,
        last_l2_ts: u64,
        current_flashblock_idx: u64,
        current_l2_idx: u64,
        now_ms: u64,
    ) {
        // Only update if we have valid samples
        if let Some(fb_sample) = self.try_sample_flashblock(
            last_flashblock_idx,
            last_flashblock_ts,
            current_flashblock_idx,
            now_ms,
        ) {
            self.update_fb_estimate(fb_sample);
        }

        if let Some(l2_sample) = self.try_sample_l2(last_l2_idx, last_l2_ts, current_l2_idx, now_ms)
        {
            self.update_l2_estimate(l2_sample);
        }
    }

    /// Updates the EMA estimate of the flashblock duration.
    fn update_fb_estimate(&self, sample_ms: u64) -> u64 {
        let old = self.fb_estimate_f64.load(Ordering::Relaxed);
        let new = self.fb_alpha * (sample_ms as f64) + (1.0 - self.fb_alpha) * old;
        self.fb_estimate_f64.store(new, Ordering::Relaxed);
        new.max(1.0).round() as u64
    }

    /// Updates the EMA estimate of the L2 block duration.
    fn update_l2_estimate(&self, sample_ms: u64) -> u64 {
        let old = self.l2_estimate_f64.load(Ordering::Relaxed);
        let new = self.l2_alpha * (sample_ms as f64) + (1.0 - self.l2_alpha) * old;
        self.l2_estimate_f64.store(new, Ordering::Relaxed);
        new.max(1.0).round() as u64
    }

    /// Downsamples observed flashblock durations and checks validity.
    ///
    /// The flashblock is valid if:
    /// - The previous flashblock index and timestamp are valid.
    /// - The current flashblock index is greater than the previous flashblock
    ///   index.
    /// - The current flashblock index is not zero. This is because the first
    ///   flashblock is special in that it does not contain user txns, so we
    ///   ignore it.
    ///
    /// Returns the sample if the flashblock is valid.
    fn try_sample_flashblock(
        &self,
        last_flashblock_idx: u64,
        last_flashblock_ts: u64,
        current_flashblock_idx: u64,
        now_ms: u64,
    ) -> Option<u64> {
        if last_flashblock_idx == 0 || last_flashblock_ts == 0 || current_flashblock_idx == 0 {
            return None;
        }

        let delta = current_flashblock_idx.saturating_sub(last_flashblock_idx);
        if delta == 0 {
            return None;
        }
        let dt = now_ms.saturating_sub(last_flashblock_ts);
        let per_fb = dt / delta;
        Some(per_fb)
    }

    /// A L2 block is valid if:
    /// - The previous L2 block index and timestamp are valid.
    /// - The current L2 block index is greater than the previous L2 block
    ///   index.
    ///
    /// Returns the sample if the L2 block is valid.
    fn try_sample_l2(
        &self,
        last_l2_idx: u64,
        last_l2_ts: u64,
        current_l2_idx: u64,
        now_ms: u64,
    ) -> Option<u64> {
        if last_l2_idx == 0 || last_l2_ts == 0 {
            return None;
        }

        if current_l2_idx > last_l2_idx {
            let delta = current_l2_idx - last_l2_idx;
            let dt_total = now_ms.saturating_sub(last_l2_ts);
            let per_l2 = dt_total / delta;
            Some(per_l2)
        } else {
            None
        }
    }
}

/// Compute EMA alpha from window length via 2/(N+1).
fn alpha_from_window(window: usize) -> f64 {
    let n = window.max(1) as f64;
    2.0 / (n + 1.0)
}
