//! Helpers for the OKX Market Maker API

use super::api_types::OkxPricingQueryParams;
use crate::error::AuthServerError;

/// Parse a chain ID from a query string
pub fn parse_chain_id(query_str: &str) -> Result<u64, AuthServerError> {
    if query_str.is_empty() {
        return Err(AuthServerError::bad_request("Missing chainIndex query parameter"));
    }

    let params: OkxPricingQueryParams = serde_urlencoded::from_str(query_str)
        .map_err(|_| AuthServerError::bad_request("Invalid query string format"))?;

    params
        .chain_index
        .ok_or_else(|| AuthServerError::bad_request("Missing chainIndex query parameter"))
}
