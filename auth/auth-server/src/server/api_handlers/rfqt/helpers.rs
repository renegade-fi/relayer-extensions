//! RFQT helpers

use auth_server_api::rfqt::RfqtLevelsQueryParams;
use renegade_common::types::chain::Chain;

use crate::error::AuthServerError;

/// Parse query string into `RfqtLevelsQueryParams` with validation against
/// server chain
pub fn parse_levels_query_params(
    query_str: &str,
    server_chain: Chain,
) -> Result<RfqtLevelsQueryParams, AuthServerError> {
    // If string is empty, return default (do nothing)
    if query_str.is_empty() {
        return Ok(RfqtLevelsQueryParams::default());
    }

    // Parse chain ID, return bad request on failure
    let chain_id = query_str
        .parse::<u64>()
        .map_err(|_| AuthServerError::bad_request("Invalid chain ID format"))?;

    // Validate chain ID matches server chain
    validate_chain_id(chain_id, server_chain)?;

    // Return validated chain ID
    Ok(RfqtLevelsQueryParams { chain_id: Some(chain_id) })
}

/// Validate that the provided chain ID matches the server's configured chain
fn validate_chain_id(provided_chain_id: u64, server_chain: Chain) -> Result<(), AuthServerError> {
    let server_chain_id = chain_to_chain_id(server_chain);
    if provided_chain_id != server_chain_id {
        return Err(AuthServerError::bad_request(format!(
            "Chain ID mismatch: expected {server_chain_id}, got {provided_chain_id}",
        )));
    }
    Ok(())
}

/// Convert a Chain enum to its numeric chain ID
fn chain_to_chain_id(chain: Chain) -> u64 {
    match chain {
        Chain::ArbitrumOne => 42161,
        Chain::ArbitrumSepolia => 421614,
        Chain::BaseMainnet => 8453,
        Chain::BaseSepolia => 84532,
        Chain::Devnet => 0,
    }
}
