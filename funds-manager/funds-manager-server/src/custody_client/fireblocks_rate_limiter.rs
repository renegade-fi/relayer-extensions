//! Rate limiter for outbound calls to the Fireblocks API.
//!
//! Fireblocks rate-limits API calls per workspace. When we exceed the
//! ceiling, requests return 429 Too Many Requests. Under sustained load a
//! 429 storm affects all calls in the workspace — including the EIP-712
//! sign calls that gardener needs to keep Hyperliquid hedges in sync.
//!
//! This limiter wraps the Fireblocks SDK at the funds-manager layer and
//! gates every outbound call through two mechanisms:
//!
//! 1. A token bucket caps steady-state RPS. Tokens refill at
//!    `STEADY_STATE_RPS`, up to `BURST_CAPACITY`. Each call consumes one
//!    token; if none are available, the call awaits the next refill.
//! 2. On 429 from any call, the limiter enters a cooldown — subsequent
//!    `acquire` calls block until the cooldown elapses. Cooldown duration
//!    grows multiplicatively per consecutive 429 (1s, 2s, 4s, 8s, 16s,
//!    capped at 30s) and resets after any successful call.
//!
//! Limitation: the Fireblocks Rust SDK's error type does not preserve
//! response headers, so we cannot read `Retry-After`. The fixed
//! multiplicative backoff above approximates it.
//!
//! Scope: a single global limiter is shared across every `CustodyClient`
//! in the process. Fireblocks quota is per-workspace, not per-chain, and
//! the funds-manager uses one API key across all chains, so a single
//! shared bucket models the actual quota correctly.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, OnceLock,
};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};

use crate::log_task;
use crate::logger::{Outcome, Task};

/// Steady-state token refill rate in requests per second.
///
/// Sized to stay below Fireblocks' observed workspace ceiling with headroom.
/// Tune downward if 429 storms persist; tune upward if signing latency
/// grows under normal load.
const STEADY_STATE_RPS: u64 = 15;

/// Maximum token bucket capacity. Permits a short burst at process start
/// or after an idle window without immediately rate-limiting.
const BURST_CAPACITY: usize = 30;

/// Initial cooldown after the first 429 in a streak.
const INITIAL_COOLDOWN_SECS: u64 = 1;

/// Hard ceiling on cooldown duration. Past this point further 429s do not
/// extend the gate further — the caller will see continued failures rather
/// than the limiter holding traffic indefinitely.
const MAX_COOLDOWN_SECS: u64 = 30;

/// A token-bucket rate limiter for Fireblocks API calls, augmented with a
/// 429-triggered cooldown gate.
pub struct FireblocksLimiter {
    /// Token bucket. Each `acquire` consumes one permit; permits are
    /// refilled by the refill task at `STEADY_STATE_RPS`. Permits acquired
    /// by callers are forgotten (not returned to the pool); only the refill
    /// task replenishes them, which is what makes this a rate limiter and
    /// not a concurrency limiter.
    permits: Arc<Semaphore>,
    /// Cap on the token bucket. The refill task does not add a token if
    /// the bucket is already at capacity.
    capacity: usize,
    /// Deadline before which `acquire` calls block, set when a 429 is
    /// observed. `None` while no cooldown is active.
    cooldown_until: Mutex<Option<Instant>>,
    /// Count of consecutive 429s; resets on any success. Drives the
    /// multiplicative backoff in [`Self::on_429`].
    consecutive_429s: AtomicU32,
}

impl FireblocksLimiter {
    /// Construct a new limiter, spawning the background refill task. The
    /// task is detached and runs for the lifetime of the limiter.
    pub fn new(steady_state_rps: u64, burst_capacity: usize) -> Arc<Self> {
        let me = Arc::new(Self {
            permits: Arc::new(Semaphore::new(burst_capacity)),
            capacity: burst_capacity,
            cooldown_until: Mutex::new(None),
            consecutive_429s: AtomicU32::new(0),
        });

        let weak = Arc::downgrade(&me);
        let refill_interval = Duration::from_nanos(1_000_000_000 / steady_state_rps);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(refill_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(lim) = weak.upgrade() else { break };
                if lim.permits.available_permits() < lim.capacity {
                    lim.permits.add_permits(1);
                }
            }
        });

        me
    }

    /// Block until a token is available and any active cooldown has
    /// elapsed. The consumed token is not returned to the bucket; the
    /// refill task replenishes at the configured RPS.
    pub async fn acquire(&self) {
        loop {
            let deadline = *self.cooldown_until.lock().await;
            if let Some(until) = deadline {
                let now = Instant::now();
                if now < until {
                    tokio::time::sleep(until - now).await;
                    continue;
                }
            }
            break;
        }

        let permit = self
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("Fireblocks limiter semaphore should never be closed");
        permit.forget();
    }

    /// Notify the limiter that a Fireblocks call returned 429. Pushes the
    /// cooldown deadline forward (multiplicative on consecutive 429s).
    pub async fn on_429(&self) {
        let n = self.consecutive_429s.fetch_add(1, Ordering::Relaxed) + 1;
        let shift = (n.min(5) - 1) as u64;
        let secs = std::cmp::min(INITIAL_COOLDOWN_SECS << shift, MAX_COOLDOWN_SECS);
        let new_until = Instant::now() + Duration::from_secs(secs);

        let mut guard = self.cooldown_until.lock().await;
        if guard.map_or(true, |existing| existing < new_until) {
            *guard = Some(new_until);
        }
        log_task!(
            Task::FireblocksRateLimit,
            Outcome::Partial,
            cooldown_secs = secs,
            consecutive_429s = n,
            "Fireblocks 429 received; gating workspace traffic for {}s (consecutive 429s: {})",
            secs,
            n
        );
    }

    /// Notify the limiter that a Fireblocks call succeeded. Resets the
    /// consecutive-429 counter so the next 429 starts the backoff from
    /// `INITIAL_COOLDOWN_SECS` again.
    pub fn on_success(&self) {
        self.consecutive_429s.store(0, Ordering::Relaxed);
    }
}

/// The process-wide Fireblocks limiter. Lazily initialized on first
/// access — funds-manager always runs under a tokio runtime by the time
/// any custody-client method runs, so spawning the refill task from this
/// closure is safe.
static GLOBAL_LIMITER: OnceLock<Arc<FireblocksLimiter>> = OnceLock::new();

/// Get a handle to the process-wide Fireblocks limiter, initializing it
/// on first call.
pub fn global_limiter() -> Arc<FireblocksLimiter> {
    GLOBAL_LIMITER.get_or_init(|| FireblocksLimiter::new(STEADY_STATE_RPS, BURST_CAPACITY)).clone()
}

/// Trait that lets the limiter wrapper recognize a 429 across the two
/// distinct error shapes the Fireblocks Rust SDK exposes: the per-API
/// generic `apis::Error<T>` (from generated OpenAPI methods) and the
/// top-level `FireblocksError` (from hand-rolled SDK helpers like
/// `Client::addresses`).
pub trait Is429 {
    /// Returns true if the error indicates Fireblocks returned HTTP 429.
    fn is_429(&self) -> bool;
}

impl<T> Is429 for fireblocks_sdk::apis::Error<T> {
    fn is_429(&self) -> bool {
        matches!(
            self,
            fireblocks_sdk::apis::Error::ResponseError(rc) if rc.status.as_u16() == 429
        )
    }
}

impl Is429 for fireblocks_sdk::FireblocksError {
    fn is_429(&self) -> bool {
        use fireblocks_sdk::FireblocksError as E;
        match self {
            E::InternalError { code, .. }
            | E::Unknown { code, .. }
            | E::InvalidRequest { code, .. } => *code == 429,
            _ => false,
        }
    }
}
