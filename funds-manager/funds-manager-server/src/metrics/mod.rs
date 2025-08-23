//! General metrics recording functionality

use alloy::providers::DynProvider;
use alloy_primitives::Address;
use price_reporter_client::PriceReporterClient;
use renegade_common::types::chain::Chain;

use crate::{error::FundsManagerError, helpers::build_provider};

pub mod cost;
pub mod labels;

/// A general metrics recorder that holds the clients needed for recording
/// metrics.
#[derive(Clone)]
pub struct MetricsRecorder {
    /// Client for interacting with the price reporter
    pub price_reporter: PriceReporterClient,
    /// Ethereum provider for querying chain events
    pub provider: DynProvider,
    /// The chain for which metrics are being recorded
    pub chain: Chain,
    /// The address of the darkpool contract
    pub darkpool_address: Address,
}

impl MetricsRecorder {
    /// Create a new metrics recorder
    pub async fn new(
        price_reporter: PriceReporterClient,
        rpc_url: &str,
        chain: Chain,
        darkpool_address: Address,
    ) -> Result<Self, FundsManagerError> {
        let provider = build_provider(rpc_url, None /* wallet */).await?;

        Ok(MetricsRecorder { price_reporter, provider, chain, darkpool_address })
    }
}
