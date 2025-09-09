//! RFQT endpoint types

use serde::Deserialize;
use serde_json::json;

/// Query params for GET /rfqt/v3/levels
#[derive(Debug, Deserialize, Default)]
pub struct RfqtLevelsQueryParams {
    /// EVM chain id
    #[serde(rename = "chainId")]
    pub chain_id: Option<String>,
}

/// Dummy response body for GET /rfqt/v3/levels
pub fn dummy_levels_body() -> serde_json::Value {
    json!({
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48": {
            "bids": [["1600.21", "0.55"]],
            "asks": [["1601.25", "2.1"]]
        },
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2/0xdac17f958d2ee523a2206206994597c13d831ec7": {
            "bids": [["1600.25", "0.5"]],
            "asks": []
        }
    })
}

/// Parse query string into `RfqtLevelsQueryParams` with a best-effort default
pub fn parse_levels_query_params(query_str: &str) -> RfqtLevelsQueryParams {
    if query_str.is_empty() {
        RfqtLevelsQueryParams::default()
    } else {
        serde_urlencoded::from_str(query_str).unwrap_or_default()
    }
}
