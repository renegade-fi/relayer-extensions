//! Arrival controller
//!
//! Goal: choose send time `s` such that arrival is near target time `T`.
//!
//! - `delay_estimate`: predictive baseline of one-way delay (EMA over recent
//!   data)
//!
//! Send Rule: `s = T - delay_estimate`

use std::sync::{Arc, Mutex};

use crate::arrival_control::ema::Ema;

/// EMA window length for the delay estimate.
///
/// Decrease this to react quickly to changes in delay; increase to smooth out
/// noise.
pub const DELAY_WINDOW: u32 = 12;

/// A thread-safe arrival controller.
#[derive(Clone)]
pub struct ArrivalController {
    /// The EMA instance for tracking delay estimates.
    delay_ema: Arc<Mutex<Ema>>,
}

impl Default for ArrivalController {
    fn default() -> Self {
        let delay_ema = Ema::from_window_length(DELAY_WINDOW);
        Self { delay_ema: Arc::new(Mutex::new(delay_ema)) }
    }
}

impl ArrivalController {
    /// Compute local timestamp at which to send to target an arrival at
    /// `target_ms`.
    pub fn compute_send_ms(&self, target_ms: u64) -> u64 {
        let delay_estimate =
            self.delay_ema.lock().expect("EMA lock poisoned").last().unwrap_or(0.0);
        target_ms.saturating_sub(delay_estimate.round() as u64)
    }

    /// Update the delay estimate with a new observation.
    pub fn on_feedback(&self, _target_ms: u64, send_ms: u64, ack_ms: u64) {
        // Update the delay EMA with the observed delay
        self.update_delay_estimate(send_ms, ack_ms);
    }

    /// Updates the delay estimate with a new observation.
    /// We approximate the one-way delay as the time between sending and
    /// observing the packet arrival.
    fn update_delay_estimate(&self, send_ms: u64, ack_ms: u64) {
        let mut ema = self.delay_ema.lock().expect("EMA lock poisoned");

        let observed_one_way_delay_ms = ack_ms.saturating_sub(send_ms) as f64;
        let new_delay_estimate_ms = ema.update(observed_one_way_delay_ms).max(0.0);

        tracing::info!("old delay estimate: {}ms", ema.last().unwrap());
        tracing::info!("observed delay: {}ms", observed_one_way_delay_ms);
        tracing::info!("new delay estimate: {}ms", new_delay_estimate_ms);
    }
}
