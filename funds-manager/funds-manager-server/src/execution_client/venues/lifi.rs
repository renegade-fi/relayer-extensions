//! Lifi-specific logic for getting quotes and executing swaps.
//!
//! Includes definitions for the Lifi API types, as defined in
//! <https://apidocs.li.fi/reference/get_v1-quote>

use alloy::hex;
use alloy_primitives::{Address, Bytes, U256};
use renegade_common::types::{chain::Chain, token::Token};
use serde::Deserialize;

use crate::execution_client::{
    error::ExecutionClientError,
    venues::{
        quote::{ExecutableQuote, ExecutionQuote, QuoteExecutionData},
        SupportedExecutionVenue,
    },
};

/// Transaction request details from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionRequest {
    /// Destination contract address
    to: String,
    /// Hex-encoded calldata for the transaction
    data: String,
    /// Amount of native token to send (in hex)
    value: String,
    /// Gas limit in hex
    gas_limit: String,
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
    transaction_request: TransactionRequest,
    /// Quote estimate details
    estimate: Estimate,
    /// Swap action details
    action: Action,
    /// Tool (venue) providing the route
    tool: String,
}

impl LifiQuote {
    /// Get the token being sold
    fn get_sell_token(&self, chain: Chain) -> Token {
        Token::from_addr_on_chain(&self.action.from_token.address, chain)
    }

    /// Get the token being bought
    fn get_buy_token(&self, chain: Chain) -> Token {
        Token::from_addr_on_chain(&self.action.to_token.address, chain)
    }

    /// Get the amount of tokens being sold
    fn get_sell_amount(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(&self.estimate.from_amount, 10)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the amount of tokens being bought
    fn get_buy_amount(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(&self.estimate.to_amount, 10)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the address of the swap contract that will be called
    fn get_to_address(&self) -> Result<Address, ExecutionClientError> {
        self.transaction_request.to.parse().map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the address of the submitting address
    fn get_from_address(&self) -> Result<Address, ExecutionClientError> {
        self.action.from_address.parse().map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the value of the tx; should be zero
    fn get_value(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(self.transaction_request.value.trim_start_matches("0x"), 16)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the calldata for the swap
    fn get_data(&self) -> Result<Bytes, ExecutionClientError> {
        hex::decode(self.transaction_request.data.trim_start_matches("0x"))
            .map_err(ExecutionClientError::quote_conversion)
            .map(Bytes::from)
    }

    /// Get the gas limit for the swap
    fn get_gas_limit(&self) -> Result<U256, ExecutionClientError> {
        U256::from_str_radix(self.transaction_request.gas_limit.trim_start_matches("0x"), 16)
            .map_err(ExecutionClientError::quote_conversion)
    }

    /// Get the tool (venue) providing the route
    fn get_tool(&self) -> String {
        self.tool.clone()
    }
}

/// Lifi-specific quote execution data
#[derive(Debug, Clone)]
pub struct LifiQuoteExecutionData {
    /// The swap contract address
    pub to: Address,
    /// The submitting address
    pub from: Address,
    /// The value of the tx; should be zero
    pub value: U256,
    /// The calldata for the swap
    pub data: Bytes,
    /// The gas limit for the swap
    pub gas_limit: U256,
    /// The tool (venue) providing the route
    pub tool: String,
}

impl ExecutableQuote {
    /// Convert a LiFi quote into an executable quote
    pub fn from_lifi_quote(
        lifi_quote: LifiQuote,
        chain: Chain,
    ) -> Result<Self, ExecutionClientError> {
        let sell_token = lifi_quote.get_sell_token(chain);
        let buy_token = lifi_quote.get_buy_token(chain);
        let sell_amount = lifi_quote.get_sell_amount()?;
        let buy_amount = lifi_quote.get_buy_amount()?;

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Lifi,
            chain,
        };

        let to = lifi_quote.get_to_address()?;
        let from = lifi_quote.get_from_address()?;
        let value = lifi_quote.get_value()?;
        let data = lifi_quote.get_data()?;
        let gas_limit = lifi_quote.get_gas_limit()?;
        let tool = lifi_quote.get_tool();

        let execution_data = LifiQuoteExecutionData { to, from, value, data, gas_limit, tool };

        Ok(ExecutableQuote { quote, execution_data: QuoteExecutionData::Lifi(execution_data) })
    }
}
