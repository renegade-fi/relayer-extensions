//! A per-user rate limiter for gas sponsorship

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::Mutex;

// -------------
// | Constants |
// -------------

/// One day duration
const ONE_DAY: Duration = Duration::from_days(1);

// ---------
// | Types |
// ---------

/// A type alias for a per-user gas sponsorship rate limiter
type GasSponsorshipBucketMap = HashMap<String, GasSponsorshipBucket>;
/// A type alias for a shared gas sponsorship bucket map
type SharedGasSponsorshipBucketMap = Arc<Mutex<GasSponsorshipBucketMap>>;

// ----------------
// | Rate Limiter |
// ----------------

/// A per-user gas sponsorship rate limiter.
#[derive(Clone)]
pub struct GasSponsorshipRateLimiter {
    /// The maximum dollar value of gas sponsorship funds per day.
    ///
    /// We currently use the same value for all users.
    max_value: f64,
    /// The bucket map for per-user gas sponsorship buckets
    buckets: SharedGasSponsorshipBucketMap,
}

impl GasSponsorshipRateLimiter {
    /// Create a new gas sponsorship rate limiter
    pub fn new(max_value: f64) -> Self {
        Self { max_value, buckets: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Create a new gas sponsorship bucket for a given user
    fn new_bucket(&self) -> GasSponsorshipBucket {
        GasSponsorshipBucket::new(self.max_value, ONE_DAY)
    }

    /// Get the remaining value and time for a given user's bucket.
    pub async fn remaining_value_and_time(&self, user_id: String) -> (f64, Duration) {
        let mut map = self.buckets.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_bucket());
        (entry.remaining_value(), entry.remaining_time())
    }

    /// Check if the given user's bucket has a non-zero remaining value.
    pub async fn has_remaining_value(&self, user_id: String) -> bool {
        let mut map = self.buckets.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_bucket());
        entry.has_remaining_value()
    }

    /// Record a gas sponsorship value for a given user.
    ///
    /// If the user does not have any remaining gas sponsorship budget, this
    /// method will do nothing.
    pub async fn record_sponsorship(&self, user_id: String, value: f64) {
        let mut map = self.buckets.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_bucket());
        entry.record_sponsorship(value);
    }
}

/// A single user's gas sponsorship bucket.
///
/// This struct does *not* use atomics, rather it is assumed that access to the
/// bucket will be properly synchronized by the caller.
pub struct GasSponsorshipBucket {
    /// Maximum dollar value of gas sponsorship funds in the bucket
    max_value: f64,
    /// Dollar value of remaining gas sponsorship funds in the bucket
    remaining_value: f64,
    /// Interval at which the bucket is refilled
    refill_interval: Duration,
    /// The next refill time
    next_refill: Instant,
}

impl GasSponsorshipBucket {
    /// Create a new gas sponsorship bucket
    pub fn new(max_value: f64, refill_interval: Duration) -> Self {
        Self {
            max_value,
            remaining_value: max_value,
            refill_interval,
            next_refill: Instant::now() + refill_interval,
        }
    }

    // -----------
    // | Getters |
    // -----------

    /// Get the remaining value in the bucket.
    pub fn remaining_value(&self) -> f64 {
        self.remaining_value
    }

    /// Get the remaining time in the bucket.
    pub fn remaining_time(&self) -> Duration {
        self.next_refill - Instant::now()
    }

    // -----------
    // | Setters |
    // -----------

    /// Check if the bucket has a non-zero remaining value.
    ///
    /// Note that we do not check if the bucket can cover a specific sponsorship
    /// value - if there is any remaining value in the bucket, this method will
    /// return true.
    pub fn has_remaining_value(&mut self) -> bool {
        // Potentially refill the bucket, if it is due
        self.try_refill();

        // Return true if there is any remaining value
        self.remaining_value > 0.0
    }

    /// Record a gas sponsorship value.
    ///
    /// If the bucket does not have any remaining value, this method will do
    /// nothing
    pub fn record_sponsorship(&mut self, value: f64) {
        if !self.has_remaining_value() {
            return;
        }

        // Record the sponsorship value. It is okay for this operation to cause the
        // `remaining_value` to be negative.
        self.remaining_value -= value;
    }

    // -----------
    // | Helpers |
    // -----------

    /// Attempt to refill the bucket, if it is due.
    fn try_refill(&mut self) {
        let now = Instant::now();

        if now < self.next_refill {
            return;
        }

        self.remaining_value = self.max_value;
        self.next_refill = now + self.refill_interval;
    }
}
