//! General metrics recording functionality

use ethers::prelude::*;
use std::sync::Arc;

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
    pub provider: Arc<Provider<Http>>,
}

impl MetricsRecorder {
    /// Create a new metrics recorder
    pub fn new(relayer_client: RelayerClient, rpc_url: String) -> Self {
        let provider = Provider::<Http>::try_from(rpc_url).unwrap();
        let provider = Arc::new(provider);

        MetricsRecorder { relayer_client, provider }
    }
}
