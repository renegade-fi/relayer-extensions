//! Metric recording helpers

use renegade_common::types::{exchange::Exchange, token::Token};

use crate::utils::PairInfo;

// -------------
// | Constants |
// -------------

/// The prefix to apply to all metrics emitted by the price reporter
pub const METRICS_PREFIX: &str = "price-reporter";

/// The name of the metric counting the number of attempts to establish an
/// exchange connection
const EXCHANGE_CONNECTION_ATTEMPTS_METRIC_NAME: &str = "exchange_connection_attempts";
/// The name of the metric tracking the latency of a price update
const PRICE_UPDATE_LATENCY_METRIC_NAME: &str = "price_update_latency_ms";

/// The tag for the exchange
const EXCHANGE_TAG: &str = "exchange";
/// The tag for the asset
const ASSET_TAG: &str = "asset";

/// Increment the metric tracking the number of attempts to establish an
/// exchange connection
pub fn increment_exchange_connection_attempts(exchange: Exchange, base_token: &Token) {
    let ticker = base_token.get_ticker().unwrap_or(base_token.get_addr());
    let tags =
        vec![(EXCHANGE_TAG.to_string(), exchange.to_string()), (ASSET_TAG.to_string(), ticker)];

    metrics::counter!(EXCHANGE_CONNECTION_ATTEMPTS_METRIC_NAME, &tags).increment(1);
}

/// Record a metric tracking the latency (in ms) of a price update for the given
/// pair info
pub fn record_price_update_latency(pair_info: &PairInfo, latency_ms: f64) {
    let base_token = pair_info.base_token();
    let ticker = base_token.get_ticker().unwrap_or(base_token.get_addr());
    let exchange = pair_info.exchange.to_string();

    let tags = vec![(EXCHANGE_TAG.to_string(), exchange), (ASSET_TAG.to_string(), ticker)];

    metrics::gauge!(PRICE_UPDATE_LATENCY_METRIC_NAME, &tags).set(latency_ms);
}
