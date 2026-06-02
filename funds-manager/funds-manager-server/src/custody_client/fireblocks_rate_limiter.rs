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
//!    `STEADY_STATE_RPS`, up to `BURST_CAPACITY`. Each call consumes one token;
//!    if none are available, the call awaits the next refill.
//! 2. On 429 from any call, the limiter enters a cooldown — subsequent
//!    `acquire` calls block until the cooldown elapses.
//!
//! Cooldown duration is the LONGER of two values: a multiplicative backoff
//! (1s, 2s, 4s, 8s, 16s, capped at 30s, keyed on the consecutive-429 streak
//! and reset after any successful call) and any `Retry-After` captured by
//! [`super::fireblocks_retry_after::RetryAfterCapture`] into the
//! [`RetryAfterStore`] this limiter owns. `Retry-After` can only EXTEND the
//! cooldown (up to the 30s ceiling), never shorten it below the backoff: a
//! too-short or bogus server value must not pull the gate down and hammer an
//! already-overloaded Fireblocks. Absent/unparseable header ⇒ pure backoff.
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

/// Side channel for `Retry-After` durations observed by the
/// `RetryAfterCapture` middleware. The middleware records the parsed value
/// here on every 429; the limiter consumes it in [`FireblocksLimiter::on_429`]
/// to set the cooldown to exactly what Fireblocks asked for. Multiple
/// captures coalesce by keeping the latest (largest expiry), so back-to-back
/// 429s on different in-flight requests don't shorten the cooldown.
pub struct RetryAfterStore {
    deadline: std::sync::Mutex<Option<Instant>>,
}

impl RetryAfterStore {
    fn new() -> Arc<Self> {
        Arc::new(Self { deadline: std::sync::Mutex::new(None) })
    }

    /// Record a `Retry-After` window observed on a 429 response. If a
    /// previous window is still in the future, the later of the two wins
    /// (Fireblocks is allowed to extend a cooldown but never shorten one
    /// we already promised the bucket).
    pub fn record(&self, duration: Duration) {
        let new_until = Instant::now() + duration;
        let mut guard = self.deadline.lock().unwrap();
        if guard.is_none_or(|existing| existing < new_until) {
            *guard = Some(new_until);
        }
    }

    /// Take any pending Retry-After deadline, clearing the store. Returns
    /// `None` if no deadline was captured since the last consumer call.
    fn take(&self) -> Option<Instant> {
        self.deadline.lock().unwrap().take()
    }
}

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
    /// `Retry-After` durations published by the middleware that observes
    /// raw Fireblocks responses. Consulted first in [`Self::on_429`]; when
    /// the store is empty, the limiter falls back to multiplicative backoff.
    retry_after: Arc<RetryAfterStore>,
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
            retry_after: RetryAfterStore::new(),
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

    /// Notify the limiter that a Fireblocks call returned 429. Sets the
    /// cooldown to the longer of the multiplicative backoff (1s, 2s, 4s, 8s,
    /// 16s, capped at 30s, keyed on the consecutive-429 streak) and any
    /// captured `Retry-After`. `Retry-After` can only extend the wait, never
    /// shorten it below the backoff floor, and is clamped to the 30s ceiling;
    /// a too-short value is ignored in favor of the backoff.
    pub async fn on_429(&self) {
        let n = self.consecutive_429s.fetch_add(1, Ordering::Relaxed) + 1;

        // Multiplicative backoff for this streak: 1,2,4,8,16s capped at MAX.
        let shift = (n.min(5) - 1) as u64;
        let backoff_secs = std::cmp::min(INITIAL_COOLDOWN_SECS << shift, MAX_COOLDOWN_SECS);

        // `Retry-After` may only EXTEND the cooldown, never shorten it below
        // the backoff floor. A too-short or bogus value (e.g. 0-1s while the
        // co-signer is deeply backed up) would otherwise let us retry early
        // and intensify the overload, so we floor at `backoff_secs` and clamp
        // to the 30s ceiling.
        let (secs, source) = match self.retry_after.take() {
            Some(deadline) => {
                let ra_secs = deadline.saturating_duration_since(Instant::now()).as_secs();
                if ra_secs > backoff_secs {
                    (ra_secs.min(MAX_COOLDOWN_SECS), "retry_after_header")
                } else {
                    (backoff_secs, "retry_after_floored")
                }
            },
            None => (backoff_secs, "multiplicative_backoff"),
        };
        let new_until = Instant::now() + Duration::from_secs(secs);

        let mut guard = self.cooldown_until.lock().await;
        if guard.is_none_or(|existing| existing < new_until) {
            *guard = Some(new_until);
        }
        log_task!(
            Task::FireblocksRateLimit,
            Outcome::Partial,
            cooldown_secs = secs,
            consecutive_429s = n,
            source = source,
            "Fireblocks 429 received; gating workspace traffic for {}s (source: {}, consecutive 429s: {})",
            secs,
            source,
            n
        );
    }

    /// Handle to the limiter's [`RetryAfterStore`]. Hand this to the
    /// `RetryAfterCapture` middleware at SDK construction so middleware
    /// writes flow directly into the bucket this limiter consumes.
    pub fn retry_after_store(&self) -> Arc<RetryAfterStore> {
        self.retry_after.clone()
    }

    /// Notify the limiter that a Fireblocks call succeeded. Resets the
    /// consecutive-429 counter so the next 429 starts the backoff from
    /// `INITIAL_COOLDOWN_SECS` again.
    pub fn on_success(&self) {
        self.consecutive_429s.store(0, Ordering::Relaxed);
    }
}

/// The Fireblocks API user a call is billed to. Fireblocks sets rate limits
/// at the API-user level (per endpoint, per minute), so each user gets its
/// own process-global limiter with an independent budget.
#[derive(Clone, Copy, Debug)]
pub enum FireblocksUserClass {
    /// Signing user — POST /v1/transactions (the latency-critical path).
    Signing,
    /// Polling user — GET transaction status reads.
    Polling,
    /// Read user — vault / asset / wallet info reads.
    Read,
}

/// The process-wide Fireblocks limiters, one per API user. Lazily initialized
/// on first access — funds-manager always runs under a tokio runtime by the
/// time any custody-client method runs, so spawning the refill task from these
/// closures is safe. They are process-global (not per-`FireblocksClient`)
/// because the budget is per-API-user and shared across every chain's client
/// in this process.
static SIGNING_LIMITER: OnceLock<Arc<FireblocksLimiter>> = OnceLock::new();
static POLLING_LIMITER: OnceLock<Arc<FireblocksLimiter>> = OnceLock::new();
static READ_LIMITER: OnceLock<Arc<FireblocksLimiter>> = OnceLock::new();

/// Get a handle to the process-wide Fireblocks limiter for `class`,
/// initializing it on first call.
pub fn global_limiter(class: FireblocksUserClass) -> Arc<FireblocksLimiter> {
    let cell = match class {
        FireblocksUserClass::Signing => &SIGNING_LIMITER,
        FireblocksUserClass::Polling => &POLLING_LIMITER,
        FireblocksUserClass::Read => &READ_LIMITER,
    };
    cell.get_or_init(|| FireblocksLimiter::new(STEADY_STATE_RPS, BURST_CAPACITY)).clone()
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
