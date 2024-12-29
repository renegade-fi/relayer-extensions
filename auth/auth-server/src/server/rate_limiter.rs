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

/// The number of bundles allowed per minute
const BUNDLES_LIMIT_PER_MINUTE: u64 = 4;
/// One minute duration
const ONE_MINUTE: Duration = Duration::from_secs(60);

/// The bundle rate limiter
#[derive(Clone)]
pub struct BundleRateLimiter {
    /// A per-user rate limiter
    bucket_map: Arc<Mutex<BucketMap>>,
}

impl BundleRateLimiter {
    /// Create a new bundle rate limiter
    pub fn new() -> Self {
        Self { bucket_map: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Create a new rate limiter
    fn new_rate_limiter() -> Ratelimiter {
        Ratelimiter::builder(BUNDLES_LIMIT_PER_MINUTE, ONE_MINUTE)
            .initial_available(BUNDLES_LIMIT_PER_MINUTE)
            .max_tokens(BUNDLES_LIMIT_PER_MINUTE)
            .build()
            .expect("invalid rate limit configuration")
    }

    /// Consume a token from bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn check(&self, user_id: String) -> bool {
        let mut map = self.bucket_map.lock().await;
        let entry = map.entry(user_id).or_insert_with(Self::new_rate_limiter);

        let available = entry.available();
        entry.set_available(available.saturating_sub(1)).expect("rate limit range should be valid");
        available >= 1
    }

    /// Increment the number of tokens available to a given user
    #[allow(unused_must_use)]
    pub async fn add_token(&self, user_id: String) {
        let mut map = self.bucket_map.lock().await;
        let entry = map.entry(user_id).or_insert_with(Self::new_rate_limiter);

        // Set the available tokens
        // The underlying rate limiter will error if this exceeds the configured
        // maximum, we ignore this error
        let available = entry.available();
        entry.set_available(available + 1);
    }
}
