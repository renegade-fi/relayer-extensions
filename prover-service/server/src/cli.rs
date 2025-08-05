//! CLI for the prover service

use renegade_util::telemetry::{configure_telemetry_with_metrics_config, metrics::MetricsConfig};

use crate::error::ProverServiceError;

/// The prefix for all metrics emitted by the service
const METRICS_PREFIX: &str = "prover_service";
/// The statsd host to use for metrics
const DEFAULT_STATSD_HOST: &str = "127.0.0.1";
/// The statsd port to use for metrics
const DEFAULT_STATSD_PORT: u16 = 8125;
/// The default OTLP collector endpoint
const DEFAULT_OTLP_COLLECTOR_ENDPOINT: &str = "http://localhost:4317";

/// The CLI for the prover service
#[derive(Debug, clap::Parser)]
pub struct Cli {
    /// The port to listen on
    #[clap(short, long, default_value = "3000", env = "PORT")]
    pub port: u16,
    /// The HTTP basic auth password
    #[clap(long, env = "HTTP_AUTH_PASSWORD")]
    pub auth_password: String,

    // --- Telemetry --- //
    /// Whether or not to enable Datadog-formatted logs
    #[arg(long, env = "ENABLE_DATADOG")]
    pub datadog_enabled: bool,
}

impl Cli {
    /// Configure telemetry for the service from the arguments passed in
    pub fn configure_telemetry(&self) -> Result<(), ProverServiceError> {
        let metrics_config =
            MetricsConfig { metrics_prefix: METRICS_PREFIX.to_string(), ..Default::default() };

        // We use the single `datadog_enabled` flag for all telemetry here
        configure_telemetry_with_metrics_config(
            self.datadog_enabled,                        // datadog_enabled
            self.datadog_enabled,                        // otlp_enabled
            self.datadog_enabled,                        // metrics_enabled
            DEFAULT_OTLP_COLLECTOR_ENDPOINT.to_string(), // collector_endpoint
            DEFAULT_STATSD_HOST,                         // statsd_host
            DEFAULT_STATSD_PORT,                         // statsd_port
            Some(metrics_config),
        )
        .map_err(ProverServiceError::setup)
    }
}
