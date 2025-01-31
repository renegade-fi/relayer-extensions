//! A per-user token bucket rate limiter, used for managing quote and bundle
//! rate limit tokens

use std::{collections::HashMap, sync::Arc, time::Duration};

use ratelimit::Ratelimiter;
use tokio::sync::Mutex;

// -------------
// | Constants |
// -------------

/// One minute duration
const ONE_MINUTE: Duration = Duration::from_secs(60);

// ---------
// | Types |
// ---------

/// A type alias for a per-user rate limiter
type BucketMap = HashMap<String, Ratelimiter>;
/// A type alias for a shared bucket map
type SharedBucketMap = Arc<Mutex<BucketMap>>;

// ----------------
// | Rate Limiter |
// ----------------

/// A per user token bucket rate limiter
#[derive(Clone)]
pub struct ApiTokenRateLimiter {
    /// The number of tokens allowed per minute
    rate_limit: u64,
    /// The token buckets in a per-user map
    buckets: SharedBucketMap,
}

impl ApiTokenRateLimiter {
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
