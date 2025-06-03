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

use std::time::Duration;

use gas_sponsorship_rate_limiter::GasSponsorshipRateLimiter;
use tracing::{instrument, warn};
use user_rate_limiter::ApiTokenRateLimiter;

use crate::error::AuthServerError;

use super::Server;

mod gas_sponsorship_rate_limiter;
mod user_rate_limiter;

// -----------------------------
// | Server Rate Limit Methods |
// -----------------------------

impl Server {
    /// Check the quote rate limiter
    #[instrument(skip(self))]
    pub async fn check_quote_rate_limit(
        &self,
        key_description: String,
    ) -> Result<(), AuthServerError> {
        if !self.rate_limiter.check_quote_token(key_description.clone()).await {
            warn!("Quote rate limit exceeded for key: {key_description}");
            return Err(AuthServerError::RateLimit);
        }
        Ok(())
    }

    /// Check the bundle rate limiter
    #[instrument(skip(self))]
    pub async fn check_bundle_rate_limit(
        &self,
        key_description: String,
        shared: bool,
    ) -> Result<(), AuthServerError> {
        if !self.rate_limiter.check_bundle_token(key_description.clone(), shared).await {
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
    pub async fn check_gas_sponsorship_rate_limit(&self, key_description: &str) -> bool {
        if !self.rate_limiter.check_gas_sponsorship(key_description).await {
            warn!(
                key_description = key_description,
                "Gas sponsorship rate limit exceeded for key: {key_description}"
            );
            return false;
        }
        true
    }
}

// ----------------
// | Rate Limiter |
// ----------------

/// The bundle rate limiter
#[derive(Clone)]
pub struct AuthServerRateLimiter {
    /// The quote rate limiter
    quote_rate_limiter: ApiTokenRateLimiter,
    /// The bundle rate limiter
    bundle_rate_limiter: ApiTokenRateLimiter,
    /// The shared bundle rate limiter
    shared_bundle_rate_limiter: ApiTokenRateLimiter,
    /// The gas sponsorship rate limiter
    gas_sponsorship_rate_limiter: GasSponsorshipRateLimiter,
}

impl AuthServerRateLimiter {
    /// Create a new bundle rate limiter
    pub fn new(
        quote_rate_limit: u64,
        bundle_rate_limit: u64,
        shared_bundle_rate_limit: u64,
        max_gas_sponsorship_value: f64,
    ) -> Self {
        Self {
            quote_rate_limiter: ApiTokenRateLimiter::new(quote_rate_limit),
            bundle_rate_limiter: ApiTokenRateLimiter::new(bundle_rate_limit),
            shared_bundle_rate_limiter: ApiTokenRateLimiter::new(shared_bundle_rate_limit),
            gas_sponsorship_rate_limiter: GasSponsorshipRateLimiter::new(max_gas_sponsorship_value),
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
    pub async fn check_bundle_token(&self, user_id: String, shared: bool) -> bool {
        if shared {
            self.shared_bundle_rate_limiter.check(user_id).await
        } else {
            self.bundle_rate_limiter.check(user_id).await
        }
    }

    /// Increment the number of tokens available to a given user
    #[allow(unused_must_use)]
    pub async fn add_bundle_token(&self, user_id: String, shared: bool) {
        if shared {
            self.shared_bundle_rate_limiter.add_token(user_id).await;
        } else {
            self.bundle_rate_limiter.add_token(user_id).await;
        }
    }

    /// Check if the given user has any remaining gas sponsorship budget
    pub async fn check_gas_sponsorship(&self, user_id: &str) -> bool {
        self.gas_sponsorship_rate_limiter.has_remaining_value(user_id).await
    }

    /// Record a gas sponsorship value for a given user.
    ///
    /// If the user does not have any remaining gas sponsorship budget, this
    /// method will do nothing.
    pub async fn record_gas_sponsorship(&self, user_id: String, value: f64) {
        self.gas_sponsorship_rate_limiter.record_sponsorship(user_id, value).await;
    }

    /// Get the remaining value and time for a given user's gas sponsorship
    /// bucket.
    pub async fn remaining_gas_sponsorship_value_and_time(
        &self,
        user_id: String,
    ) -> (f64, Duration) {
        self.gas_sponsorship_rate_limiter.remaining_value_and_time(user_id).await
    }
}
