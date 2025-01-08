//! A rate limiter for the server
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

/// One minute duration
const ONE_MINUTE: Duration = Duration::from_secs(60);

/// The bundle rate limiter
#[derive(Clone)]
pub struct BundleRateLimiter {
    /// The number of bundles allowed per minute
    rate_limit: u64,
    /// A per-user rate limiter
    bucket_map: Arc<Mutex<BucketMap>>,
}

impl BundleRateLimiter {
    /// Create a new bundle rate limiter
    pub fn new(rate_limit: u64) -> Self {
        Self { rate_limit, bucket_map: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Create a new rate limiter
    fn new_rate_limiter(&self) -> Ratelimiter {
        Ratelimiter::builder(self.rate_limit, ONE_MINUTE)
            .initial_available(self.rate_limit)
            .max_tokens(self.rate_limit)
            .build()
            .expect("invalid rate limit configuration")
    }

    /// Consume a token from bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn check(&self, user_id: String) -> bool {
        let mut map = self.bucket_map.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_rate_limiter());

        entry.try_wait().is_ok()
    }

    /// Increment the number of tokens available to a given user
    #[allow(unused_must_use)]
    pub async fn add_token(&self, user_id: String) {
        let mut map = self.bucket_map.lock().await;
        let entry = map.entry(user_id).or_insert_with(|| self.new_rate_limiter());

        // Set the available tokens
        // The underlying rate limiter will error if this exceeds the configured
        // maximum, we ignore this error
        let available = entry.available();
        entry.set_available(available + 1);
    }
}
