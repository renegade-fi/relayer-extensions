//! `reqwest_middleware` layer that captures `Retry-After` headers from
//! Fireblocks 429 responses into the rate limiter's [`RetryAfterStore`].
//!
//! Installed via the SDK's `ClientBuilder::with_middleware`, this middleware
//! observes every raw Fireblocks response — before the SDK strips headers
//! while building its typed error variants — and forwards parsed
//! `Retry-After` durations into the limiter so [`FireblocksLimiter::on_429`]
//! can honor server-directed backoff exactly instead of approximating with
//! a fixed multiplicative schedule.
//!
//! Parsing is restricted to the integer "delta-seconds" form documented by
//! Fireblocks. The HTTP-date form defined by RFC 7231 is unused in practice;
//! if Fireblocks ever sends one we drop it on the floor and the limiter
//! falls back to its multiplicative backoff (which is strictly safe — the
//! caller's worst case is the pre-existing behavior).

use std::sync::Arc;

use async_trait::async_trait;
use http1::Extensions;
use reqwest_middleware::reqwest::{header::RETRY_AFTER, Request, Response, StatusCode};
use reqwest_middleware::{Middleware, Next, Result};
use std::time::Duration;

use super::fireblocks_rate_limiter::RetryAfterStore;

/// Middleware that publishes any `Retry-After` window observed on a 429
/// into a shared [`RetryAfterStore`].
pub struct RetryAfterCapture {
    store: Arc<RetryAfterStore>,
}

impl RetryAfterCapture {
    pub fn new(store: Arc<RetryAfterStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Middleware for RetryAfterCapture {
    async fn handle(
        &self,
        req: Request,
        ext: &mut Extensions,
        next: Next<'_>,
    ) -> Result<Response> {
        let resp = next.run(req, ext).await?;
        if resp.status() == StatusCode::TOO_MANY_REQUESTS {
            if let Some(duration) = resp.headers().get(RETRY_AFTER).and_then(parse_retry_after) {
                self.store.record(duration);
            }
        }
        Ok(resp)
    }
}

/// Parse the `Retry-After` header value as an integer number of seconds.
/// Returns `None` for the HTTP-date form or any unparseable value — the
/// limiter's multiplicative fallback covers those cases.
fn parse_retry_after(value: &reqwest_middleware::reqwest::header::HeaderValue) -> Option<Duration> {
    value.to_str().ok()?.trim().parse::<u64>().ok().map(Duration::from_secs)
}
