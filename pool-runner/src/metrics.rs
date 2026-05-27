//! Metrics definitions and helpers for the pool-runner service

use metrics::{counter, histogram};

// --- Metric names --- //

pub const ORDER_REASSIGN_ATTEMPT: &str = "pool_runner_order_reassign_attempt";
pub const ORDER_REASSIGN_SUCCESS: &str = "pool_runner_order_reassign_success";
pub const ORDER_REASSIGN_FAILURE: &str = "pool_runner_order_reassign_failure";
pub const ORDER_REASSIGN_TIMEOUT: &str = "pool_runner_order_reassign_timeout";
pub const FILL_LATENCY_MS: &str = "pool_runner_fill_latency_ms";

// --- Recording helpers --- //

/// Record a reassignment attempt for the given pool
pub fn record_reassign_attempt(pool: &str) {
    counter!(ORDER_REASSIGN_ATTEMPT, "matching_pool" => pool.to_string()).increment(1);
}

/// Record a successful reassignment for the given pool
pub fn record_reassign_success(pool: &str) {
    counter!(ORDER_REASSIGN_SUCCESS, "matching_pool" => pool.to_string()).increment(1);
}

/// Record a reassignment failure for the given pool
pub fn record_reassign_failure(pool: &str, reason: &str) {
    counter!(
        ORDER_REASSIGN_FAILURE,
        "matching_pool" => pool.to_string(),
        "reason" => reason.to_string()
    )
    .increment(1);
}

/// Record a reassignment timeout for the given pool
pub fn record_reassign_timeout(pool: &str) {
    counter!(ORDER_REASSIGN_TIMEOUT, "matching_pool" => pool.to_string()).increment(1);
}

/// Record the fill latency (ms) for the given pool
pub fn record_fill_latency(pool: &str, latency_ms: f64) {
    histogram!(FILL_LATENCY_MS, "matching_pool" => pool.to_string()).record(latency_ms);
}
