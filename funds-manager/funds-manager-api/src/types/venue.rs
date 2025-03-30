//! Types specific to execution venue (LiFi) integration
//! as defined in https://apidocs.li.fi/reference/get_v1-quote
use ethers::types::{Bytes, U256};
use serde::Deserialize;

use super::quoters::ExecutionQuote;

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
    /// Gas price in hex
    gas_price: String,
}

/// Gas cost information from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GasCost {
    /// Estimated gas cost
    estimate: String,
}

/// Quote estimate details from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Estimate {
    /// List of gas costs for the transaction
    gas_costs: Vec<GasCost>,
    /// Amount of tokens to sell (including decimals)
    from_amount: String,
    /// Amount of tokens to receive (including decimals)
    to_amount: String,
    /// Minimum amount of tokens to receive (including decimals)
    to_amount_min: String,
}

/// Token information from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Token {
    /// Token contract address
    address: String,
}

/// Swap action details from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Action {
    /// Token being sold
    from_token: Token,
    /// Token being bought
    to_token: Token,
    /// Address initiating the swap
    from_address: String,
}

/// Raw quote response structure from LiFi API
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiFiQuote {
    /// Transaction request details
    transaction_request: TransactionRequest,
    /// Quote estimate details
    estimate: Estimate,
    /// Swap action details
    action: Action,
}

impl From<LiFiQuote> for ExecutionQuote {
    fn from(quote: LiFiQuote) -> Self {
        let buy_token_address = quote.action.to_token.address.parse().unwrap();
        let sell_token_address = quote.action.from_token.address.parse().unwrap();
        let from = quote.action.from_address.parse().unwrap();
        let to = quote.transaction_request.to.parse().unwrap();

        let sell_amount = U256::from_str_radix(&quote.estimate.from_amount, 10).unwrap();
        let buy_amount = U256::from_str_radix(&quote.estimate.to_amount, 10).unwrap();
        let buy_amount_min = U256::from_str_radix(&quote.estimate.to_amount_min, 10).unwrap();

        let value =
            U256::from_str_radix(quote.transaction_request.value.trim_start_matches("0x"), 16)
                .unwrap();
        let gas_price =
            U256::from_str_radix(quote.transaction_request.gas_price.trim_start_matches("0x"), 16)
                .unwrap();
        let estimated_gas =
            U256::from_str_radix(&quote.estimate.gas_costs[0].estimate, 10).unwrap();

        let data = hex::decode(quote.transaction_request.data.trim_start_matches("0x"))
            .map(Bytes::from)
            .unwrap();

        ExecutionQuote {
            buy_token_address,
            sell_token_address,
            sell_amount,
            buy_amount,
            buy_amount_min,
            from,
            to,
            data,
            value,
            gas_price,
            estimated_gas,
        }
    }
}
