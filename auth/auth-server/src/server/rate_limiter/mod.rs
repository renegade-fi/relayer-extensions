//! A rate limiter for the server
//!
//! We rate limit via three different mechanisms:
//! - Quote tokens: These are used for quote requests and typically have a high
//!   max tokens value. A quote is purely informational, and therefore does not
//!   require active liquidity.
//! - Bundle tokens: These are used for bundle requests and typically have a low
//!   max tokens value. A bundle indicates an intent to trade, and therefore
//!   requires active liquidity.
//! - Gas sponsorship: This is used for sponsored match bundles. We keep track
//!   of approximate dollar value of sponsorship when a sponsored bundle is
//!   settled.
//!
//! For the first two mechanisms, the unit which we rate limit is number of
//! inflight bundles. Therefore, there are two ways for the token bucket to
//! refill:
//!     - Wait for the next refill
//!     - Settle a bundle on-chain
//!
//! The latter is measured by waiting for nullifier spend events on-chain. This
//! is also when we record the gas sponsorship value for sponsored bundles.

use chrono::TimeDelta;
use redis::aio::ConnectionManager as RedisConnection;
use tracing::{error, instrument, warn};
use uuid::Uuid;

use crate::{
    error::AuthServerError,
    server::{
        db::{create_redis_client, models::RateLimitMethod},
        rate_limiter::{
            execution_cost_rate_limiter::ExecutionCostRateLimiter,
            redis_rate_limiter::RedisRateLimiter,
        },
    },
};

use super::Server;

mod execution_cost_rate_limiter;
mod redis_rate_limiter;

/// The bundle rate limiter key prefix
const BUNDLE_RATE_LIMITER_KEY_PREFIX: &str = "bundle_rate_limit";
/// The quote rate limiter key prefix
const QUOTE_RATE_LIMITER_KEY_PREFIX: &str = "quote_rate_limit";
/// The gas sponsorship rate limiter key prefix
const GAS_SPONSORSHIP_RATE_LIMITER_KEY_PREFIX: &str = "gas_sponsorship_rate_limit";

/// One minute time delta
const ONE_MINUTE: TimeDelta = TimeDelta::minutes(1);
/// One day time delta
const ONE_DAY: TimeDelta = TimeDelta::days(1);

// -----------------------------
// | Server Rate Limit Methods |
// -----------------------------

impl Server {
    /// Consume a quote rate limit token
    ///
    /// Returns an error if the rate limit has been exceeded
    #[instrument(skip(self))]
    pub async fn consume_quote_rate_limit_token(
        &self,
        key_id: Uuid,
        key_description: &str,
    ) -> Result<(), AuthServerError> {
        let max_tokens = self.get_rate_limit(key_id, RateLimitMethod::Quote).await?;
        if !self.rate_limiter.consume_quote_token(key_description, max_tokens).await {
            warn!("Quote rate limit exceeded for key: {key_description}");
            return Err(AuthServerError::RateLimit);
        }
        Ok(())
    }

    /// Consume a bundle rate limit token
    ///
    /// Returns an error if the rate limit has been exceeded
    #[instrument(skip(self))]
    pub async fn consume_bundle_rate_limit_token(
        &self,
        key_id: Uuid,
        key_description: &str,
    ) -> Result<(), AuthServerError> {
        let max_tokens = self.get_rate_limit(key_id, RateLimitMethod::Assemble).await?;
        if !self.rate_limiter.consume_bundle_token(key_description, max_tokens).await {
            warn!("Bundle rate limit exceeded for key: {key_description}");
            return Err(AuthServerError::RateLimit);
        }
        Ok(())
    }

    /// Peek at the bundle rate limit
    ///
    /// Returns an error if the rate limit is exceeded
    #[instrument(skip(self))]
    pub async fn peek_bundle_rate_limit(
        &self,
        key_id: Uuid,
        key_description: &str,
    ) -> Result<(), AuthServerError> {
        let max_tokens = self.get_rate_limit(key_id, RateLimitMethod::Assemble).await?;
        if self.rate_limiter.check_bundle_rate_limit(key_description, max_tokens).await {
            warn!("Bundle rate limit exceeded for key: {key_description}");
            return Err(AuthServerError::RateLimit);
        }

        Ok(())
    }

    /// Check the gas sponsorship rate limiter
    ///
    /// Returns a boolean indicating whether or not the gas sponsorship rate
    /// limit has been exceeded.
    #[instrument(skip(self))]
    pub async fn check_gas_sponsorship_rate_limit(
        &self,
        key_description: &str,
    ) -> Result<bool, AuthServerError> {
        if !self.rate_limiter.check_gas_sponsorship(key_description).await? {
            warn!(
                key_description = key_description,
                "Gas sponsorship rate limit exceeded for key: {key_description}"
            );
            return Ok(false);
        }
        Ok(true)
    }

    /// Check the execution cost rate limiter
    #[instrument(skip(self))]
    pub async fn check_execution_cost_exceeded(&self, ticker: &str) -> bool {
        self.rate_limiter.check_execution_cost_exceeded(ticker).await
    }
}

// ----------------
// | Rate Limiter |
// ----------------

/// The bundle rate limiter
#[derive(Clone)]
pub struct AuthServerRateLimiter {
    /// The quote rate limiter
    quote_rate_limiter: RedisRateLimiter,
    /// The bundle rate limiter
    bundle_rate_limiter: RedisRateLimiter,
    /// The gas sponsorship rate limiter
    gas_sponsorship_rate_limiter: RedisRateLimiter,
    /// The execution cost rate limiter
    execution_cost_rate_limiter: ExecutionCostRateLimiter,
}

impl AuthServerRateLimiter {
    /// Create a new bundle rate limiter
    pub async fn new(
        quote_rate_limit: u64,
        bundle_rate_limit: u64,
        max_gas_sponsorship_value: f64,
        auth_server_redis_url: &str,
        execution_cost_redis_url: &str,
    ) -> Result<Self, AuthServerError> {
        // Create the rate limiters
        let conn = create_redis_client(auth_server_redis_url).await?;
        let quote_rate_limiter = Self::new_quote_rate_limiter(quote_rate_limit, conn.clone());
        let bundle_rate_limiter = Self::new_bundle_rate_limiter(bundle_rate_limit, conn.clone());
        let gas_sponsorship_rate_limiter =
            Self::new_gas_sponsorship_rate_limiter(max_gas_sponsorship_value, conn);
        let execution_cost_rate_limiter =
            ExecutionCostRateLimiter::new(execution_cost_redis_url).await?;

        // Load the rate limit scripts, this only needs to be called on one of the rate
        // limiters.
        quote_rate_limiter.load_scripts().await?;
        Ok(Self {
            quote_rate_limiter,
            bundle_rate_limiter,
            gas_sponsorship_rate_limiter,
            execution_cost_rate_limiter,
        })
    }

    /// Create a new quote rate limiter
    pub fn new_quote_rate_limiter(max_tokens: u64, conn: RedisConnection) -> RedisRateLimiter {
        RedisRateLimiter::new(
            QUOTE_RATE_LIMITER_KEY_PREFIX.to_string(),
            max_tokens as f64,
            ONE_MINUTE,
            conn,
        )
    }

    /// Create a new bundle rate limiter
    pub fn new_bundle_rate_limiter(max_tokens: u64, conn: RedisConnection) -> RedisRateLimiter {
        RedisRateLimiter::new(
            BUNDLE_RATE_LIMITER_KEY_PREFIX.to_string(),
            max_tokens as f64,
            ONE_MINUTE,
            conn,
        )
    }

    /// Create a new gas sponsorship rate limiter
    pub fn new_gas_sponsorship_rate_limiter(
        max_value: f64,
        conn: RedisConnection,
    ) -> RedisRateLimiter {
        RedisRateLimiter::new(
            GAS_SPONSORSHIP_RATE_LIMITER_KEY_PREFIX.to_string(),
            max_value,
            ONE_DAY,
            conn,
        )
    }

    // ----------------------
    // | Rate Limit Methods |
    // ----------------------

    /// Consume a quote token from bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn consume_quote_token(&self, user_id: &str, max_tokens: Option<u32>) -> bool {
        let max_tokens = max_tokens.map(|t| t as f64);
        match self.quote_rate_limiter.increment_consumed(user_id, 1.0, max_tokens).await {
            Ok(_) => true,
            Err(AuthServerError::RateLimit) => false,
            Err(e) => {
                error!("Error incrementing quote token: {e}");
                false
            },
        }
    }

    /// Consume a bundle token from bucket if available
    ///
    /// If no token is available (rate limit reached), this method returns
    /// false, otherwise true
    pub async fn consume_bundle_token(&self, user_id: &str, max_tokens: Option<u32>) -> bool {
        let max_tokens = max_tokens.map(|t| t as f64);
        match self.bundle_rate_limiter.increment_consumed(user_id, 1.0, max_tokens).await {
            Ok(_) => true,
            Err(AuthServerError::RateLimit) => false,
            Err(e) => {
                error!("Error incrementing bundle token: {e}");
                false
            },
        }
    }

    /// Check the bundle rate limit
    ///
    /// Returns true if the rate limit is exceeded otherwise false
    pub async fn check_bundle_rate_limit(&self, user_id: &str, max_tokens: Option<u32>) -> bool {
        let max_tokens = max_tokens.map(|t| t as f64);
        self.bundle_rate_limiter.rate_limit_exceeded(user_id, max_tokens).await.unwrap_or(false)
    }

    /// Increment the number of tokens available to a given user
    pub async fn add_bundle_token(&self, user_id: &str) -> Result<(), AuthServerError> {
        self.bundle_rate_limiter.decrement_consumed(user_id, 1.0).await.map(|_| ())
    }

    /// Check if the given user has any remaining gas sponsorship budget
    pub async fn check_gas_sponsorship(&self, user_id: &str) -> Result<bool, AuthServerError> {
        let exceeded = self
            .gas_sponsorship_rate_limiter
            .rate_limit_exceeded(user_id, None /* max_tokens */)
            .await?;
        Ok(!exceeded)
    }

    /// Record a gas sponsorship value for a given user.
    ///
    /// If the user does not have any remaining gas sponsorship budget, this
    /// method will do nothing.
    pub async fn record_gas_sponsorship(
        &self,
        user_id: &str,
        value: f64,
    ) -> Result<(), AuthServerError> {
        self.gas_sponsorship_rate_limiter.increment_consumed_no_check(user_id, value).await?;
        Ok(())
    }

    /// Check if execution costs have been exceeded for the given ticker
    ///
    /// Returns false if the rate limit has been exceeded, otherwise true
    ///
    /// We make this method infallible for now to prevent bugs here blocking the
    /// API entirely
    pub async fn check_execution_cost_exceeded(&self, ticker: &str) -> bool {
        match self.execution_cost_rate_limiter.rate_limit_exceeded(ticker).await {
            Ok(exceeded) => exceeded,
            Err(e) => {
                // If we fail to check the rate limit, assume it has been exceeded
                error!("Error checking execution cost rate limit: {e}");
                true
            },
        }
    }
}
