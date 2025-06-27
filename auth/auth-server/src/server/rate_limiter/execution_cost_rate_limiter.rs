//! A rate limiter that routes external match flow based on whether the bot
//! server's execution costs have been exceeded

use chrono::Utc;
use redis::{aio::ConnectionManager as RedisConnection, AsyncCommands};

use crate::{error::AuthServerError, server::db::create_redis_client};

/// A rate limiter that measures the bot server's per-asset execution costs and
/// rate limits external match flow that crosses with quoter orders
#[derive(Clone)]
pub struct ExecutionCostRateLimiter {
    /// The Redis connection manager
    redis: RedisConnection,
}

impl ExecutionCostRateLimiter {
    /// Constructor
    pub async fn new(redis_url: &str) -> Result<Self, AuthServerError> {
        let conn = create_redis_client(redis_url).await?;
        Ok(Self { redis: conn })
    }

    /// Check if the rate limit has been exceeded for the given ticker
    pub async fn rate_limit_exceeded(&self, ticker: &str) -> Result<bool, AuthServerError> {
        let key = Self::get_rate_limit_exceeded_key(ticker);
        let result: Option<bool> = self.redis().get(key).await?;
        Ok(result.unwrap_or(false))
    }

    // -----------
    // | Helpers |
    // -----------

    /// Get the Redis connection manager
    fn redis(&self) -> RedisConnection {
        self.redis.clone()
    }

    /// Get the rate limit exceeded key for the given ticker
    fn get_rate_limit_exceeded_key(ticker: &str) -> String {
        let date = Self::get_current_date_string();
        format!("execution_cost_exceeded:{ticker}#{date}")
    }

    /// Get the current date string in the format YYYY-MM-DD
    fn get_current_date_string() -> String {
        let now = Utc::now();
        now.format("%Y-%m-%d").to_string()
    }
}
