//! EMA manager for flash(block) duration and residual forecast error.

use super::constants::{DEFAULT_FLASHBLOCK_MS, DEFAULT_L2_MS, FLASHBLOCK_WINDOW, L2_WINDOW};
use crate::{
    arrival_control::ema::Ema,
    flashblocks::clock::constants::{DEFAULT_FORECAST_ERROR_MS, FORECAST_ERROR_WINDOW},
};

/// EMA manager for flash(block) duration and residual forecast error.
pub(crate) struct EmaManager {
    /// EMA for the flashblock duration.
    flashblock_duration: Ema,
    /// EMA for the L2 block duration.
    block_duration: Ema,
    /// EMA for the websocket forecast error.
    forecast_error: Ema,
}

impl EmaManager {
    /// Creates a new EMA manager.
    pub fn new() -> Self {
        Self {
            flashblock_duration: Ema::from_window_length(FLASHBLOCK_WINDOW, DEFAULT_FLASHBLOCK_MS),
            block_duration: Ema::from_window_length(L2_WINDOW, DEFAULT_L2_MS),
            forecast_error: Ema::from_window_length(
                FORECAST_ERROR_WINDOW,
                DEFAULT_FORECAST_ERROR_MS,
            ),
        }
    }

    /// Returns the current EMA estimate of the flashblock duration.
    ///
    /// Note: this rounds to the nearest millisecond.
    pub fn flashblock_duration_ms(&self) -> u64 {
        let est = self.flashblock_duration.last();
        est.round() as u64
    }

    /// Returns the current EMA estimate of the L2 block duration.
    ///
    /// Note: this rounds to the nearest millisecond.
    pub fn l2_block_duration_ms(&self) -> u64 {
        let est = self.block_duration.last();
        est.round() as u64
    }

    /// Returns the current EMA estimate of the websocket forecast error.
    pub fn forecast_error_ms(&self) -> f64 {
        self.forecast_error.last()
    }

    /// Updates the EMA estimate of the flashblock duration.
    pub fn update_fb_estimate(&self, sample_ms: u64) {
        self.flashblock_duration.update(sample_ms as f64);
    }

    /// Updates the EMA estimate of the L2 block duration.
    pub fn update_l2_estimate(&self, sample_ms: u64) {
        self.block_duration.update(sample_ms as f64);
    }

    /// Updates the EMA estimate of the websocket forecast error.
    ///
    /// The websocket forecast error is the difference between the actual
    /// timestamp and the predicted timestamp.
    pub fn update_forecast_error_estimate(&self, actual_ts: f64, predicted_ts: f64) {
        let sample_ts = actual_ts - predicted_ts;
        self.forecast_error.update(sample_ts);
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
    pub fn maybe_sample_flashblock_duration(
        &self,
        last_flashblock_idx: u64,
        last_flashblock_ts: u64,
        current_flashblock_idx: u64,
        current_flashblock_ts: u64,
    ) -> Option<u64> {
        if last_flashblock_idx == 0 || last_flashblock_ts == 0 || current_flashblock_idx == 0 {
            return None;
        }

        let idx_delta = current_flashblock_idx.saturating_sub(last_flashblock_idx);
        if idx_delta == 0 {
            return None;
        }
        let ts_delta = current_flashblock_ts.saturating_sub(last_flashblock_ts);
        let per_fb = ts_delta / idx_delta;
        Some(per_fb)
    }

    /// A L2 block is valid if:
    /// - The previous L2 block index and timestamp are valid.
    /// - The current L2 block index is greater than the previous L2 block
    ///   index.
    ///
    /// Returns the sample if the L2 block is valid.
    pub fn maybe_sample_l2_duration(
        &self,
        last_l2_idx: u64,
        last_l2_ts: u64,
        current_l2_idx: u64,
        current_l2_ts: u64,
    ) -> Option<u64> {
        if last_l2_idx == 0 || last_l2_ts == 0 {
            return None;
        }

        if current_l2_idx > last_l2_idx {
            let idx_delta = current_l2_idx - last_l2_idx;
            let ts_delta = current_l2_ts.saturating_sub(last_l2_ts);
            let per_l2 = ts_delta / idx_delta;
            Some(per_l2)
        } else {
            None
        }
    }
}
