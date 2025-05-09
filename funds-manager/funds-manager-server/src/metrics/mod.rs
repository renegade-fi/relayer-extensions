//! General metrics recording functionality

use alloy::providers::{DynProvider, ProviderBuilder};

use crate::relayer_client::RelayerClient;

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
        let url = rpc_url.parse().expect("invalid RPC URL");
        let provider = ProviderBuilder::new().on_http(url);
        let dyn_provider = DynProvider::new(provider);

        MetricsRecorder { relayer_client, provider: dyn_provider }
    }
}
