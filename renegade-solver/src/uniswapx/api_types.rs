//! API types for the UniswapX API
//!
//! These types are based on the UniswapX API OpenAPI specification
//! [here](https://github.com/Uniswap/uniswapx-service/blob/main/swagger.json)

use serde::{Deserialize, Serialize};

/// The response from the orders endpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetOrdersResponse {
    /// List of orders
    pub orders: Vec<OrderEntity>,
    /// Cursor for pagination (optional)
    pub cursor: Option<String>,
}

/// A UniswapX order entity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderEntity {
    /// The order type
    #[serde(rename = "type")]
    pub order_type: OrderType,
    /// Current status of the order
    pub order_status: OrderStatus,
    /// The order signature
    pub signature: String,
    /// The encoded order data
    pub encoded_order: String,
    /// The chain ID where the order exists
    pub chain_id: u64,
    /// The nonce
    pub nonce: String,
    /// The unique hash of the order
    pub order_hash: String,
    /// The swapper address (who created the order)
    pub swapper: String,
    /// Block when auction starts
    pub auction_start_block: u64,
    /// Baseline priority fee in wei
    pub baseline_priority_fee_wei: String,
    /// Input token information
    pub input: OrderInput,
    /// Output token information (array)
    pub outputs: Vec<OrderOutput>,
    /// Cosigner data
    pub cosigner_data: CosignerData,
    /// Cosignature
    pub cosignature: String,
    /// Optional quote ID
    pub quote_id: Option<String>,
    /// Timestamp when the order was created
    pub created_at: u64,
    /// Routing information
    pub route: Route,
}

/// Order input information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderInput {
    /// Token contract address
    pub token: String,
    /// Token amount
    pub amount: String,
    /// MPS per priority fee wei
    pub mps_per_priority_fee_wei: String,
}

/// Order output information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderOutput {
    /// Token contract address
    pub token: String,
    /// Token amount
    pub amount: String,
    /// MPS per priority fee wei
    pub mps_per_priority_fee_wei: String,
    /// Recipient address
    pub recipient: String,
}

/// Cosigner data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CosignerData {
    /// Auction target block
    pub auction_target_block: u64,
}

/// Route information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    /// Gas use estimate
    pub gas_use_estimate: String,
    /// Gas use estimate quote
    pub gas_use_estimate_quote: String,
    /// Gas price in wei
    pub gas_price_wei: String,
    /// Quote amount
    pub quote: String,
    /// Gas adjusted quote
    pub quote_gas_adjusted: String,
    /// Method parameters
    pub method_parameters: MethodParameters,
}

/// Method parameters for routing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodParameters {
    /// Calldata for the transaction
    pub calldata: String,
    /// Value to send with transaction
    pub value: String,
    /// Target address
    pub to: String,
}

/// Order type enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderType {
    /// Dutch auction order
    Dutch,
    /// Priority order
    Priority,
}

/// Order status enumeration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderStatus {
    /// Order is open and can be filled
    Open,
    /// Order has expired
    Expired,
    /// Order has encountered an error
    Error,
    /// Order has been cancelled
    Cancelled,
    /// Order has been filled
    Filled,
    /// Order has insufficient funds
    #[serde(rename = "insufficient-funds")]
    InsufficientFunds,
}
