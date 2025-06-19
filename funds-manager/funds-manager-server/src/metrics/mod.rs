//! General metrics recording functionality

use std::sync::Arc;

use alloy::providers::DynProvider;
use price_reporter_client::PriceReporterClient;
use renegade_common::types::chain::Chain;

use crate::helpers::build_provider;

pub mod cost;
pub mod labels;

/// A general metrics recorder that holds the clients needed for recording
/// metrics.
#[derive(Clone)]
pub struct MetricsRecorder {
    /// Client for interacting with the price reporter
    pub price_reporter: Arc<PriceReporterClient>,
    /// Ethereum provider for querying chain events
    pub provider: DynProvider,
    /// The chain for which metrics are being recorded
    pub chain: Chain,
}

impl MetricsRecorder {
    /// Create a new metrics recorder
    pub fn new(price_reporter: Arc<PriceReporterClient>, rpc_url: &str, chain: Chain) -> Self {
        let provider = build_provider(rpc_url).expect("invalid RPC URL");

        MetricsRecorder { price_reporter, provider, chain }
    }
}
