//! A rate limiter for the server
//!
//! We rate limit on two different token schedules:
//! - Quote tokens: These are used for quote requests and typically have a high
//!   max tokens value. A quote is purely informational, and therefore does not
//!   require active liquidity.
//! - Bundle tokens: These are used for bundle requests and typically have a low
//!   max tokens value. A bundle indicates an intent to trade, and therefore
//!   requires active liquidity.
//!
//! The unit which we rate limit is number of inflight bundles. Therefore, there
//! are two ways for the token bucket to refill:
//!     - Wait for the next refill
//!     - Settle a bundle on-chain
//!
//! The latter is measured by waiting for nullifier spend events on-chain

use std::{collections::HashMap, sync::Arc, time::Duration};

use ratelimit::Ratelimiter;
use tokio::sync::Mutex;

/// A type alias for a per-user rate limiter
type BucketMap = HashMap<String, Ratelimiter>;
/// A type alias for a shared bucket map
type SharedBucketMap = Arc<Mutex<BucketMap>>;

/// One minute duration
const ONE_MINUTE: Duration = Duration::from_secs(60);

/// The bundle rate limiter
#[derive(Clone)]
pub struct AuthServerRateLimiter {
    /// The quote rate limiter
    quote_rate_limiter: UserRateLimiter,
    /// The bundle rate limiter
    bundle_rate_limiter: UserRateLimiter,
}

impl AuthServerRateLimiter {
    /// Create a new bundle rate limiter
    pub fn new(quote_rate_limit: u64, bundle_rate_limit: u64) -> Self {
        Self {
            quote_rate_limiter: UserRateLimiter::new(quote_rate_limit),
            bundle_rate_limiter: UserRateLimiter::new(bundle_rate_limit),
        }
    }

    /// Consume a quote token from bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn check_quote_token(&self, user_id: String) -> bool {
        self.quote_rate_limiter.check(user_id).await
    }

    /// Consume a bundle token from bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn check_bundle_token(&self, user_id: String) -> bool {
        self.bundle_rate_limiter.check(user_id).await
    }

    /// Increment the number of tokens available to a given user
    #[allow(unused_must_use)]
    pub async fn add_bundle_token(&self, user_id: String) {
        self.bundle_rate_limiter.add_token(user_id).await;
    }
}

/// A per user token bucket rate limiter
#[derive(Clone)]
pub struct UserRateLimiter {
    /// The number of tokens allowed per minute
    rate_limit: u64,
    /// The token buckets in a per-user map
    buckets: SharedBucketMap,
}

impl UserRateLimiter {
    /// Create a new user rate limiter
    pub fn new(rate_limit: u64) -> Self {
        Self { rate_limit, buckets: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Create a new rate limiter
    fn new_rate_limiter(&self) -> Ratelimiter {
        Ratelimiter::builder(self.rate_limit, ONE_MINUTE)
            .initial_available(self.rate_limit)
            .max_tokens(self.rate_limit)
            .build()
            .expect("invalid rate limit configuration")
    }

    /// Consume a token from the bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn check(&self, user_id: String) -> bool {
        let mut map = self.buckets.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_rate_limiter());
        entry.try_wait().is_ok()
    }

    /// Increment the number of tokens available to a given user
    #[allow(unused_must_use)]
    pub async fn add_token(&self, user_id: String) {
        let mut map = self.buckets.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_rate_limiter());

        // Set the available tokens
        // The underlying rate limiter will error if this exceeds the configured
        // maximum, we ignore this error
        let available = entry.available();
        entry.set_available(available + 1);
    }
}
