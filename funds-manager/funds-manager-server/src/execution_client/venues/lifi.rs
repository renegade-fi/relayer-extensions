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
    venues::{ExecutableQuote, ExecutionQuote, QuoteExecutionData, SupportedExecutionVenue},
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

/// Lifi-specific quote execution data
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
        let sell_token = Token::from_addr_on_chain(&lifi_quote.action.from_token.address, chain);
        let buy_token = Token::from_addr_on_chain(&lifi_quote.action.to_token.address, chain);

        let sell_amount = U256::from_str_radix(&lifi_quote.estimate.from_amount, 10)
            .map_err(ExecutionClientError::quote_conversion)?;

        let buy_amount = U256::from_str_radix(&lifi_quote.estimate.to_amount, 10)
            .map_err(ExecutionClientError::quote_conversion)?;

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Lifi,
            chain,
        };

        let to = lifi_quote
            .transaction_request
            .to
            .parse()
            .map_err(ExecutionClientError::quote_conversion)?;

        let from = lifi_quote
            .action
            .from_address
            .parse()
            .map_err(ExecutionClientError::quote_conversion)?;

        let value =
            U256::from_str_radix(lifi_quote.transaction_request.value.trim_start_matches("0x"), 16)
                .map_err(ExecutionClientError::quote_conversion)?;

        let data = hex::decode(lifi_quote.transaction_request.data.trim_start_matches("0x"))
            .map_err(ExecutionClientError::quote_conversion)?
            .into();

        let gas_limit = U256::from_str_radix(
            lifi_quote.transaction_request.gas_limit.trim_start_matches("0x"),
            16,
        )
        .map_err(ExecutionClientError::quote_conversion)?;

        let tool = lifi_quote.tool;

        let execution_data = LifiQuoteExecutionData { to, from, value, data, gas_limit, tool };

        Ok(ExecutableQuote { quote, execution_data: QuoteExecutionData::Lifi(execution_data) })
    }
}
