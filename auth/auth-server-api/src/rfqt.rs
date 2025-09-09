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
#[derive(Debug)]
pub struct Level {
    /// Price (nominal amount in decimal form)
    pub price: String,
    /// Amount (nominal amount in decimal form)
    pub amount: String,
}

impl serde::Serialize for Level {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeTuple;
        let mut tuple = serializer.serialize_tuple(2)?;
        tuple.serialize_element(&self.price)?;
        tuple.serialize_element(&self.amount)?;
        tuple.end()
    }
}

// --------------------
// | Quote Endpoint   |
// --------------------

/// Request body for POST /rfqt/v3/quote
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqtQuoteRequest {
    /// Chain identifier
    pub chain_id: String,
    /// Maker token address
    pub maker_token: String,
    /// Taker token address
    pub taker_token: String,
    /// Units of taker token that the taker is offering (alternative to
    /// maker_amount)
    pub taker_amount: String,
    /// Retail end user address (must match counterparty in response)
    pub taker: String,
    /// Number used to prevent order from being filled twice
    pub nonce: String,
    /// Whether this signed order may be partially filled
    pub partial_fill_allowed: bool,
    /// 0x settlement smart contract address
    pub spender: String,
    /// Request identifier (matches 0x-zid header)
    pub zid: String,
    /// App ID (cuid, not uuid)
    pub app_id: String,
    /// Token address that fee is based on (maker or taker token)
    pub fee_token: String,
    /// Basis points of feeToken that will be billed by 0x
    pub fee_amount_bps: String,
    /// Conversion rate from feeToken to USDC (feeTokenUsdValue / UsdcValue)
    pub fee_token_conversion_rate: String,
}

/// Response for POST /rfqt/v3/quote
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RfqtQuoteResponse {
    /// JSON representation of the created and signed settlerRfqOrder
    pub order: OrderDetails,
    /// Order signature
    pub signature: String,
    /// Same as received in request
    pub fee_token: String,
    /// Same as received in request
    pub fee_amount_bps: String,
    /// Same as received in request
    pub fee_token_conversion_rate: String,
    /// Signer wallet address
    pub maker: String,
}

/// Order details in quote response
#[derive(Debug, Serialize)]
pub struct OrderDetails {
    /// Maker token and amount (permitted field refers to maker)
    pub permitted: TokenAmount,
    /// 0x settlement smart contract address
    pub spender: String,
    /// Same as received in request
    pub nonce: String,
    /// Deadline timestamp (must be 60+ seconds from now)
    pub deadline: String,
    /// Taker details (consideration field refers to taker)
    pub consideration: Consideration,
}

/// Token and amount pair
#[derive(Debug, Serialize)]
pub struct TokenAmount {
    /// Token address
    pub token: String,
    /// Amount
    pub amount: String,
}

/// Consideration details
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Consideration {
    /// Taker token address
    pub token: String,
    /// Taker amount (must match request if partial_fill_allowed=false)
    pub amount: String,
    /// Taker address (must match request taker field)
    pub counterparty: String,
    /// Whether partial fill is allowed (must match request)
    pub partial_fill_allowed: bool,
}
