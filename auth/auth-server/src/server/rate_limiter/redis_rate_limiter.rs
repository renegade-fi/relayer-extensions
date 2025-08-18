//! A rate limiter that uses Redis to store rate limit data
//!
//! Note that this rate limiter operates on per-second granularity, so
//! sub-second rate limiting is not possible in the current implementation

use chrono::{DateTime, DurationRound, TimeDelta, Utc};
use lazy_static::lazy_static;
use redis::{AsyncCommands, Script, aio::ConnectionManager as RedisConnection};

use crate::error::AuthServerError;

// ----------------
// | Redis Script |
// ----------------

/// A Redis script for incrementing a rate limit key only if this does not
/// overflow the rate limit
const INCR_LIMIT_LUA: &str = r#"
    -- Increment the rate limit key only if this does not overflow the rate limit
    --
    -- Arguments:
    --  1. The maximum number of tokens that can be used per refill interval
    --  2. The TTL for the rate limit key
    --  3. The amount to increment the rate limit key by
    local limit = tonumber(ARGV[1])
    local ttl   = tonumber(ARGV[2])
    local delta = tonumber(ARGV[3])

    --- Fetch the existing rate limit record
    local newLimit = redis.call("INCRBYFLOAT", KEYS[1], delta)

    -- Set the TTL on first touch
    if newLimit == delta then
        redis.call("EXPIREAT", KEYS[1], ttl)
    end

    -- If the new increment would overflow the rate limit, rollback
    if newLimit > limit then
        redis.call("INCRBYFLOAT", KEYS[1], -delta)
        return -1
    else
        return newLimit
    end
"#;

lazy_static! {
    static ref INCR_LIMIT_SCRIPT: Script = Script::new(INCR_LIMIT_LUA);
}

// ----------------
// | Rate Limiter |
// ----------------

/// A rate limiter using Redis as the underlying storage
#[derive(Clone)]
pub struct RedisRateLimiter {
    /// The key prefix for the rate limiter
    key_prefix: String,
    /// The maximum number of tokens that can be used per refill interval
    max_tokens: f64,
    /// The refill interval
    refill_interval: TimeDelta,
    /// The Redis connection manager
    redis: RedisConnection,
}

impl RedisRateLimiter {
    /// Create a new Redis rate limiter
    pub fn new(
        key_prefix: String,
        max_tokens: f64,
        refill_interval: TimeDelta,
        conn: RedisConnection,
    ) -> Self {
        Self { key_prefix, max_tokens, refill_interval, redis: conn }
    }

    /// Get a handle to the Redis connection
    fn redis(&self) -> RedisConnection {
        self.redis.clone()
    }

    // --------------------
    // | Rate Limit Logic |
    // --------------------

    /// Check if the rate limit has been exceeded for a given sub-key
    ///
    /// Returns `true` if the rate limit has been exceeded, otherwise `false`
    pub async fn rate_limit_exceeded(&self, user_key: &str) -> Result<bool, AuthServerError> {
        let consumed = self.get_consumed(user_key).await?;
        let exceeded = consumed >= self.max_tokens;
        Ok(exceeded)
    }

    /// Returns the current number of rate limit tokens consumed in the current
    /// refill interval
    pub async fn get_consumed(&self, user_key: &str) -> Result<f64, AuthServerError> {
        let key = self.build_key(user_key);
        let consumed: Option<f64> = self.redis().get(key).await?;
        Ok(consumed.unwrap_or(0.))
    }

    /// Increment the number of rate limit tokens consumed in the current
    /// interval
    ///
    /// Returns the new consumed value if the rate limit is not exceeded,
    /// otherwise returns an error
    pub async fn increment_consumed(
        &self,
        user_key: &str,
        amount: f64,
    ) -> Result<f64, AuthServerError> {
        let key = self.build_key(user_key);

        // Argument order is:
        //  1. Token limit
        //  2. TTL for the rate limit key
        //  3. Amount to increment the rate limit key by
        let ttl = self.get_ttl_seconds();
        let mut invocation = INCR_LIMIT_SCRIPT.key(key);
        invocation.arg(self.max_tokens).arg(ttl).arg(amount);

        let consumed: f64 = self.redis().invoke_script(&invocation).await?;
        if consumed == -1. {
            return Err(AuthServerError::RateLimit);
        }

        Ok(consumed)
    }

    // -----------
    // | Helpers |
    // -----------

    /// Build the rate limit key for a given sub-key
    fn build_key(&self, sub_key: &str) -> String {
        let nearest_refill = self.round_to_nearest_refill(self.refill_interval);
        let nearest_refill_str = nearest_refill.format("%Y-%m-%d_%H:%M:%S").to_string();

        let key_prefix = &self.key_prefix;
        format!("{key_prefix}{sub_key}#{nearest_refill_str}")
    }

    /// Round down the current time to the nearest `refill_interval`
    fn round_to_nearest_refill(&self, refill_interval: TimeDelta) -> DateTime<Utc> {
        let now = Utc::now();
        now.duration_round(refill_interval).unwrap()
    }

    /// Get the TTL for keys used by the rate limiter
    ///
    /// This is two rate limit intervals into the future
    fn get_ttl_seconds(&self) -> i64 {
        let now = Utc::now();
        let expires_in = self.refill_interval * 2;
        let ttl_datetime = now + expires_in;
        ttl_datetime.signed_duration_since(DateTime::UNIX_EPOCH).num_seconds()
    }
}
