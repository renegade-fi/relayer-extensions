//! Periodic snapshot of funds-manager's shared-resource state.
//!
//! Emits one structured `[health-snapshot] [ok]` log line every
//! [`HEALTH_SNAPSHOT_INTERVAL`] describing the in-process state of the
//! Fireblocks rate limiter (bucket occupancy, cooldown, rolling-window
//! throttle counters).
//!
//! Companion to the gardener-side snapshot loop. Together they let
//! post-incident investigations answer "what was the rate-limit bottleneck
//! doing at minute X?" by grepping `[health-snapshot]` in Datadog instead
//! of inferring it from gaps and 429 events.

use std::time::Duration;

use crate::custody_client::fireblocks_rate_limiter::global_limiter;
use crate::log_task;
use crate::logger::{Outcome, Task};

/// Cadence of the snapshot loop. Matched to the gardener-side cadence so
/// the two services' snapshot lines can be aligned by timestamp in
/// dashboards. Drop to 15s if dashboards need finer detail.
const HEALTH_SNAPSHOT_INTERVAL: Duration = Duration::from_secs(30);

/// Spawn the periodic health-snapshot task. Detached; runs for the lifetime
/// of the process. Safe to call before `warp::serve` because the limiter is
/// lazily initialized on first access (or already initialized by any prior
/// `rate_limited` call).
pub fn spawn_health_snapshot_task() {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEALTH_SNAPSHOT_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // First tick fires immediately; skip it so the first snapshot
        // covers a real 30s window rather than 0s of activity.
        interval.tick().await;
        loop {
            interval.tick().await;
            emit_snapshot().await;
        }
    });
}

async fn emit_snapshot() {
    let fb = global_limiter().snapshot_and_reset().await;
    let cooldown_suffix = if fb.cooldown_remaining_ms > 0 {
        format!(" cooldown_ms={}", fb.cooldown_remaining_ms)
    } else {
        String::new()
    };
    log_task!(
        Task::HealthSnapshot,
        Outcome::Ok,
        fb_bucket = fb.bucket,
        fb_capacity = fb.capacity,
        fb_cooldown_ms = fb.cooldown_remaining_ms,
        fb_consecutive_429s = fb.consecutive_429s,
        fb_throttled_window = fb.throttled_window,
        fb_acquired_window = fb.acquired_window,
        fb_peak_wait_ms_window = fb.peak_wait_ms_window,
        "fb={{bucket={}/{} throttled={}/{} peak_wait_ms={} consecutive_429s={}{}}}",
        fb.bucket,
        fb.capacity,
        fb.throttled_window,
        fb.acquired_window,
        fb.peak_wait_ms_window,
        fb.consecutive_429s,
        cooldown_suffix
    );
}
