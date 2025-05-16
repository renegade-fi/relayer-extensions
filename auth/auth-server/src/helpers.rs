//! Helper methods for the auth server
use alloy::signers::local::PrivateKeySigner;
use renegade_common::types::chain::Chain;
use renegade_darkpool_client::{client::DarkpoolClientConfig, DarkpoolClient};
use std::time::Duration;

/// The interval at which we poll filter updates
const DEFAULT_BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

/// Create a darkpool client with the provided configuration
pub async fn create_darkpool_client(
    darkpool_address: String,
    chain_id: Chain,
    rpc_url: String,
) -> Result<DarkpoolClient, String> {
    // Create the client
    DarkpoolClient::new(DarkpoolClientConfig {
        darkpool_addr: darkpool_address,
        chain: chain_id,
        rpc_url,
        private_key: PrivateKeySigner::random(),
        block_polling_interval: DEFAULT_BLOCK_POLLING_INTERVAL,
    })
    .map_err(|e| format!("Failed to create darkpool client: {e}"))
}
