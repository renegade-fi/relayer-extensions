//! API types for the OKX Market Maker API

use serde::{Deserialize, Serialize};

// -----------
// | Pricing |
// -----------

/// Query parameters for OKX Market Maker API requests
#[derive(Debug, Deserialize)]
pub struct OkxPricingQueryParams {
    /// Chain index identifier
    #[serde(rename = "chainIndex")]
    pub chain_index: Option<u64>,
}

/// Response from the OKX Market Maker API
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OkxPricingResponse {
    /// Response code ("0" indicates success)
    pub code: String,
    /// Response message
    pub msg: String,
    /// Response data
    pub data: OkxPricingData,
}

/// Data payload in the OKX Market Maker API response
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OkxPricingData {
    /// Chain identifier
    pub chain_index: String,
    /// Array of level data entries for different token pairs
    pub level_data: Vec<LevelDataEntry>,
}

/// Level data entry for a token pair
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelDataEntry {
    /// Address of the taker token
    ///
    /// Specifically this is the token input by the taker
    pub taker_token_address: String,
    /// Address of the maker token
    ///
    /// Specifically this is the token output by the maker
    pub maker_token_address: String,
    /// Array of price levels, where each level is a tuple of (amount, rate)
    ///
    /// - First element: Amount the maker is willing to buy (as a decimal
    ///   string)
    /// - Second element: Exchange rate (takerTokenRate) as a decimal string
    pub levels: Vec<(String, String)>,
}
