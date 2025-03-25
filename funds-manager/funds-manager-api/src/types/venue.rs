//! Types specific to execution venue (LiFi) integration
//! as defined in https://apidocs.li.fi/reference/get_v1-quote
use ethers::types::{Bytes, U256};
use serde::Deserialize;

use super::quoters::ExecutionQuote;

/// Raw quote response structure from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LiFiQuoteResponse {
    pub transaction_request: TransactionRequest,
    pub action: Action,
    pub estimate: Estimate,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TransactionRequest {
    pub to: String,
    pub data: String,
    pub value: String,
    pub gas_price: String,
    pub gas_limit: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Action {
    pub from_token: Token,
    pub to_token: Token,
    pub from_address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Token {
    pub address: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Estimate {
    pub from_amount: String,
    pub to_amount: String,
}

impl TryFrom<serde_json::Value> for ExecutionQuote {
    type Error = String;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        // First deserialize into our LiFi response type
        let quote: LiFiQuoteResponse = serde_json::from_value(value)
            .map_err(|e| format!("Failed to parse LiFi quote: {}", e))?;

        // Then convert to our ExecutionQuote
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
