//! Arrival controller
//!
//! Goal: choose send time `s` such that arrival is near target time `T`.
//!
//! - `delay_estimate`: predictive baseline of one-way delay (EMA over recent
//!   data)
//!
//! Send Rule: `s = T - delay_estimate`

use std::sync::Arc;

use crate::arrival_control::ema::Ema;

/// EMA window length for the delay estimate.
///
/// Decrease this to react quickly to changes in delay; increase to smooth out
/// noise.
pub const DELAY_WINDOW: u32 = 12;

/// Initial seed value (ms) for the delay EMA.
pub const INITIAL_DELAY_SEED_MS: f64 = 100.0;

/// A thread-safe arrival controller.
#[derive(Clone)]
pub struct ArrivalController {
    /// The EMA instance for tracking delay estimates.
    pub delay_ema: Arc<Ema>,
}

impl Default for ArrivalController {
    fn default() -> Self {
        let delay_ema = Ema::from_window_length(DELAY_WINDOW, INITIAL_DELAY_SEED_MS);
        Self { delay_ema: Arc::new(delay_ema) }
    }
}

impl ArrivalController {
    /// Compute local timestamp at which to send to target an arrival at
    /// `target_ts`.
    pub fn compute_send_ts(&self, target_ts: u64) -> u64 {
        let lead = self.delay_ema.last();
        let lead = lead.round() as u64;
        target_ts.saturating_sub(lead)
    }

    /// Update the delay estimate with a new observation.
    pub fn on_feedback(&self, submitted_ts: u64, actual_ts: u64) {
        // Update the delay EMA with the observed delay
        self.update_delay_estimate(submitted_ts, actual_ts);
    }

    /// Updates the delay estimate with a new observation.
    /// We approximate the one-way delay as the time between sending and
    /// observing the packet arrival.
    fn update_delay_estimate(&self, submitted_ts: u64, actual_ts: u64) {
        let ema = self.delay_ema.clone();

        let observed_one_way_delay_ms = actual_ts.saturating_sub(submitted_ts) as f64;

        let _ = ema.update(observed_one_way_delay_ms).max(0.0);
    }
}
