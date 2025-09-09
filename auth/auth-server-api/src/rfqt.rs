//! RFQT endpoint types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Query params for GET /rfqt/v3/levels
#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RfqtLevelsQueryParams {
    /// Chain identifier (1=Ethereum, 137=Polygon, 42161=Arbitrum, 8453=Base)
    pub chain_id: Option<u64>,
}

// --------------------
// | Levels Endpoint  |
// --------------------

/// Response for GET /rfqt/v3/levels
#[derive(Debug, Serialize)]
pub struct RfqtLevelsResponse {
    /// Token pairs and their pricing curves (flattened into the JSON object)
    #[serde(flatten)]
    pub pairs: HashMap<String, TokenPairLevels>,
}

/// Pricing curve for a token pair
#[derive(Debug, Serialize)]
pub struct TokenPairLevels {
    /// Bid pricing curve (descending price order recommended)
    pub bids: Vec<Level>,
    /// Ask pricing curve (ascending price order recommended)
    pub asks: Vec<Level>,
}

/// Individual price/amount level
#[derive(Debug, Serialize)]
pub struct Level {
    /// Price (nominal amount in decimal form)
    pub price: String,
    /// Amount (nominal amount in decimal form)
    pub amount: String,
}

/// Dummy response body for GET /rfqt/v3/levels
pub fn dummy_levels_body() -> RfqtLevelsResponse {
    let mut pairs = HashMap::new();

    // WETH/USDC pair
    pairs.insert(
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2/0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
            .to_string(),
        TokenPairLevels {
            bids: vec![Level { price: "1600.21".to_string(), amount: "0.55".to_string() }],
            asks: vec![Level { price: "1601.25".to_string(), amount: "2.1".to_string() }],
        },
    );

    // WETH/USDT pair
    pairs.insert(
        "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2/0xdac17f958d2ee523a2206206994597c13d831ec7"
            .to_string(),
        TokenPairLevels {
            bids: vec![Level { price: "1600.25".to_string(), amount: "0.5".to_string() }],
            asks: vec![],
        },
    );

    RfqtLevelsResponse { pairs }
}
