//! Types specific to execution venue (LiFi) integration
//! as defined in https://apidocs.li.fi/reference/get_v1-quote
use ethers::types::{Bytes, U256};
use serde::Deserialize;

use super::quoters::ExecutionQuote;

/// Raw quote response structure from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiFiQuoteResponse {
    /// Transaction details including to address, calldata, value, and gas
    /// parameters
    pub transaction_request: TransactionRequest,
    /// Action details including token addresses and sender
    pub action: Action,
    /// Amount estimates for the swap
    pub estimate: Estimate,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionRequest {
    /// Destination contract address
    pub to: String,
    /// Hex-encoded calldata for the transaction
    pub data: String,
    /// Amount of native token to send (in hex)
    pub value: String,
    /// Gas price in hex
    pub gas_price: String,
    /// Gas limit in hex
    pub gas_limit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Action {
    /// Token being sold
    pub from_token: Token,
    /// Token being bought
    pub to_token: Token,
    /// Address initiating the swap
    pub from_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Token {
    /// Contract address of the token
    pub address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Estimate {
    /// Amount of tokens to sell (including decimals)
    pub from_amount: String,
    /// Amount of tokens to receive (including decimals)
    pub to_amount: String,
}

impl TryFrom<serde_json::Value> for ExecutionQuote {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        let quote: LiFiQuoteResponse = serde_json::from_value(value)
            .map_err(|e| format!("Failed to parse LiFi quote: {}", e))?;

        let buy_token_address = quote
            .action
            .to_token
            .address
            .parse()
            .map_err(|e| format!("Invalid buy token address: {}", e))?;

        let sell_token_address = quote
            .action
            .from_token
            .address
            .parse()
            .map_err(|e| format!("Invalid sell token address: {}", e))?;

        let from = quote
            .action
            .from_address
            .parse()
            .map_err(|e| format!("Invalid from address: {}", e))?;

        let to = quote
            .transaction_request
            .to
            .parse()
            .map_err(|e| format!("Invalid to address: {}", e))?;

        let sell_amount = U256::from_str_radix(&quote.estimate.from_amount, 10)
            .map_err(|e| format!("Invalid sell amount: {}", e))?;

        let buy_amount = U256::from_str_radix(&quote.estimate.to_amount, 10)
            .map_err(|e| format!("Invalid buy amount: {}", e))?;

        // Parse hex strings from transaction request
        let value =
            U256::from_str_radix(quote.transaction_request.value.trim_start_matches("0x"), 16)
                .map_err(|e| format!("Invalid value: {}", e))?;

        let gas_price =
            U256::from_str_radix(quote.transaction_request.gas_price.trim_start_matches("0x"), 16)
                .map_err(|e| format!("Invalid gas price: {}", e))?;

        let estimated_gas =
            U256::from_str_radix(quote.transaction_request.gas_limit.trim_start_matches("0x"), 16)
                .map_err(|e| format!("Invalid estimated gas: {}", e))?;

        let data = hex::decode(quote.transaction_request.data.trim_start_matches("0x"))
            .map_err(|e| format!("Invalid calldata hex: {}", e))
            .map(Bytes::from)?;

        Ok(ExecutionQuote {
            buy_token_address,
            sell_token_address,
            sell_amount,
            buy_amount,
            from,
            to,
            data,
            value,
            gas_price,
            estimated_gas,
        })
    }
}
