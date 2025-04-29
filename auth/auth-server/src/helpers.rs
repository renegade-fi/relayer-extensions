//! Helper methods for the auth server
use alloy::signers::local::PrivateKeySigner;
use renegade_arbitrum_client::{
    client::{ArbitrumClient, ArbitrumClientConfig},
    constants::Chain,
};
use std::str::FromStr;

/// The interval at which we poll filter updates
const DEFAULT_BLOCK_POLLING_INTERVAL_MS: u64 = 100;

/// The dummy private key used to instantiate the arbitrum client
///
/// We don't need any client functionality using a real private key, so instead
/// we use the key deployed by Arbitrum on local devnets
const DUMMY_PRIVATE_KEY: &str =
    "0xb6b15c8cb491557369f3c7d2c287b053eb229daa9c22138887752191c9520659";

/// Create an Arbitrum client with the provided configuration
pub async fn create_arbitrum_client(
    darkpool_address: String,
    chain_id: Chain,
    rpc_url: String,
) -> Result<ArbitrumClient, String> {
    // Parse the wallet
    let wallet = match PrivateKeySigner::from_str(DUMMY_PRIVATE_KEY) {
        Ok(wallet) => wallet,
        Err(e) => return Err(format!("Failed to parse wallet: {}", e)),
    };

    // Create the client
    ArbitrumClient::new(ArbitrumClientConfig {
        darkpool_addr: darkpool_address,
        chain: chain_id,
        rpc_url,
        private_key: wallet,
        block_polling_interval_ms: DEFAULT_BLOCK_POLLING_INTERVAL_MS,
    })
    .map_err(|e| format!("Failed to create Arbitrum client: {e}"))
}
