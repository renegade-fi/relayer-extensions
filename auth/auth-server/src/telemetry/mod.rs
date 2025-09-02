//! Defines helpers for recording metrics

use renegade_util::telemetry::{configure_telemetry_with_metrics_config, metrics::MetricsConfig};

use crate::{Cli, error::AuthServerError};
pub mod helpers;
pub mod labels;
pub mod quote_comparison;
pub mod sources;

/// The threshold beyond which to ignore a quote's fill ratio
///
/// We ignore quotes beyond this value as they're likely to be spam, or are far
/// beyond expected external match liquidity so as to be useless for telemetry.
///
/// Specified in USDC
pub const QUOTE_FILL_RATIO_IGNORE_THRESHOLD: u128 = 100_000 * 10u128.pow(6u32); // $100,000 of USDC
/// The prefix for metrics
const METRICS_PREFIX: &str = "auth-server";
/// The default OTLP collector endpoint
const DEFAULT_OTLP_COLLECTOR_ENDPOINT: &str = "http://localhost:4317";

/// Configure telemetry from the command line arguments
pub(crate) fn configure_telemtry_from_args(args: &Cli) -> Result<(), AuthServerError> {
    let metrics_config =
        MetricsConfig { metrics_prefix: METRICS_PREFIX.to_string(), ..Default::default() };
    configure_telemetry_with_metrics_config(
        args.datadog_enabled,                        // datadog_enabled
        args.otlp_enabled,                           // otlp_enabled
        args.metrics_enabled,                        // metrics_enabled
        DEFAULT_OTLP_COLLECTOR_ENDPOINT.to_string(), // collector_endpoint
        &args.statsd_host,                           // statsd_host
        args.statsd_port,                            // statsd_port
        Some(metrics_config),
    )
    .map_err(AuthServerError::setup)
}
