//! General metrics recording functionality

use alloy::providers::DynProvider;

use crate::{helpers::build_provider, relayer_client::RelayerClient};

pub mod cost;
pub mod labels;

/// A general metrics recorder that holds the clients needed for recording
/// metrics.
#[derive(Clone)]
pub struct MetricsRecorder {
    /// Client for interacting with the relayer
    pub relayer_client: RelayerClient,
    /// Ethereum provider for querying chain events
    pub provider: DynProvider,
}

impl MetricsRecorder {
    /// Create a new metrics recorder
    pub fn new(relayer_client: RelayerClient, rpc_url: String) -> Self {
        let provider = build_provider(&rpc_url).expect("invalid RPC URL");

        MetricsRecorder { relayer_client, provider }
    }
}
