//! Lifi API type definitions

use alloy_primitives::{Address, Bytes, U256};
use renegade_common::types::{chain::Chain, token::Token};
use serde::{Deserialize, Serialize};

use funds_manager_api::serialization::u256_string_serialization;

use crate::execution_client::error::ExecutionClientError;

/// The subset of Lifi quote request query parameters that we support
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifiQuoteParams {
    /// The token that should be transferred. Can be the address or the symbol
    pub from_token: String,
    /// The token that should be transferred to. Can be the address or the
    /// symbol
    pub to_token: String,
    /// The amount that should be sent including all decimals (e.g. 1000000 for
    /// 1 USDC (6 decimals))
    #[serde(with = "u256_string_serialization")]
    pub from_amount: U256,
    /// The sending wallet address
    pub from_address: String,
    /// The receiving wallet address. If none is provided, the fromAddress will
    /// be used
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_address: Option<String>,
    /// The ID of the sending chain
    pub from_chain: usize,
    /// The ID of the receiving chain
    pub to_chain: usize,
    /// The maximum allowed slippage for the transaction as a decimal value.
    /// 0.005 represents 0.5%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage: Option<f64>,
    /// The maximum price impact for the transaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price_impact: Option<f64>,
    /// Timing setting to wait for a certain amount of swap rates. In the format
    /// minWaitTime-${minWaitTimeMs}-${startingExpectedResults}-${reduceEveryMs}.
    /// Please check docs.li.fi for more details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_step_timing_strategies: Option<Vec<String>>,
    /// Which kind of route should be preferred
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// Parameter to skip transaction simulation. The quote will be returned
    /// faster but the transaction gas limit won't be accurate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_simulation: Option<bool>,
    /// List of exchanges that are allowed for this transaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_exchanges: Option<Vec<String>>,
    /// List of exchanges that are not allowed for this transaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_exchanges: Option<Vec<String>>,
}

/// Transaction request details from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiTransactionRequest {
    /// Destination contract address
    to: String,
    /// Hex-encoded calldata for the transaction
    data: String,
    /// Amount of native token to send (in hex)
    value: String,
}

/// Quote estimate details from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Estimate {
    /// Amount of tokens to sell (including decimals)
    from_amount: String,
    /// Amount of tokens to receive (including decimals)
    to_amount: String,
}

/// Token information from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LifiToken {
    /// Token contract address
    address: String,
}

/// Swap action details from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Action {
    /// Token being sold
    from_token: LifiToken,
    /// Token being bought
    to_token: LifiToken,
    /// Address initiating the swap
    from_address: String,
}

/// Raw quote response structure from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LifiQuote {
    /// Transaction request details
    transaction_request: LifiTransactionRequest,
    /// Quote estimate details
    estimate: Estimate,
    /// Swap action details
    action: Action,
    /// Tool (venue) providing the route
    tool: String,
}

impl LifiQuote {
    /// Get the token being sold
    pub fn get_sell_token(&self, chain: Chain) -> Token {
        Token::from_addr_on_chain(&self.action.from_token.address, chain)
    }

    /// Get the token being bought
    pub fn get_buy_token(&self, chain: Chain) -> Token {
        Token::from_addr_on_chain(&self.action.to_token.address, chain)
    }

    /// Get the amount of tokens being sold
    pub fn get_sell_amount(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(&self.estimate.from_amount, 10)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the amount of tokens being bought
    pub fn get_buy_amount(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(&self.estimate.to_amount, 10)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the address of the swap contract that will be called
    pub fn get_to_address(&self) -> Result<Address, ExecutionClientError> {
        self.transaction_request.to.parse().map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the address of the submitting address
    pub fn get_from_address(&self) -> Result<Address, ExecutionClientError> {
        self.action.from_address.parse().map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the value of the tx; should be zero
    pub fn get_value(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(self.transaction_request.value.trim_start_matches("0x"), 16)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the calldata for the swap
    pub fn get_data(&self) -> Result<Bytes, ExecutionClientError> {
        hex::decode(self.transaction_request.data.trim_start_matches("0x"))
            .map_err(ExecutionClientError::quote_conversion)
            .map(Bytes::from)
    }

    /// Get the tool (venue) providing the route
    pub fn get_tool(&self) -> String {
        self.tool.clone()
    }
}
