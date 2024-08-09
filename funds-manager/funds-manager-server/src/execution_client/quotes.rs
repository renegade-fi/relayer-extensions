//! Client methods for fetching quotes and prices from the execution venue

use ethers::types::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

use crate::helpers::{
    address_string_serialization, bytes_string_serialization, u256_string_serialization,
};

use super::{error::ExecutionClientError, ExecutionClient};

/// The price endpoint
const PRICE_ENDPOINT: &str = "swap/v1/price";
/// The quote endpoint
const QUOTE_ENDPOINT: &str = "swap/v1/quote";

/// The buy token url param
const BUY_TOKEN: &str = "buyToken";
/// The sell token url param
const SELL_TOKEN: &str = "sellToken";
/// The sell amount url param
const SELL_AMOUNT: &str = "sellAmount";
/// The taker address url param
const TAKER_ADDRESS: &str = "takerAddress";

/// The price response
#[derive(Debug, Deserialize)]
pub struct PriceResponse {
    /// The price
    pub price: String,
}

/// The subset of the quote response forwarded to consumers of this client
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionQuote {
    /// The token address we're buying
    #[serde(with = "address_string_serialization")]
    pub buy_token_address: Address,
    /// The token address we're selling
    #[serde(with = "address_string_serialization")]
    pub sell_token_address: Address,
    /// The amount of tokens to sell
    #[serde(with = "u256_string_serialization")]
    pub sell_amount: U256,
    /// The quoted price
    pub price: String,
    /// The submitting address
    #[serde(with = "address_string_serialization")]
    pub from: Address,
    /// The 0x swap contract address
    #[serde(with = "address_string_serialization")]
    pub to: Address,
    /// The calldata for the swap
    #[serde(with = "bytes_string_serialization")]
    pub data: Bytes,
    /// The value of the tx; should be zero
    #[serde(with = "u256_string_serialization")]
    pub value: U256,
    /// The gas price used in the swap
    #[serde(with = "u256_string_serialization")]
    pub gas_price: U256,
}

impl ExecutionClient {
    /// Fetch a price for an asset
    pub async fn get_price(
        &self,
        buy_asset: &str,
        sell_asset: &str,
        amount: u128,
    ) -> Result<f64, ExecutionClientError> {
        let amount_str = amount.to_string();
        let params =
            [(BUY_TOKEN, buy_asset), (SELL_TOKEN, sell_asset), (SELL_AMOUNT, amount_str.as_str())];

        let resp: PriceResponse = self.send_get_request(PRICE_ENDPOINT, &params).await?;
        resp.price.parse::<f64>().map_err(ExecutionClientError::parse)
    }

    /// Fetch a quote for an asset
    pub async fn get_quote(
        &self,
        buy_asset: &str,
        sell_asset: &str,
        amount: u128,
        recipient: &str,
    ) -> Result<ExecutionQuote, ExecutionClientError> {
        let amount_str = amount.to_string();
        let params = [
            (BUY_TOKEN, buy_asset),
            (SELL_TOKEN, sell_asset),
            (SELL_AMOUNT, amount_str.as_str()),
            (TAKER_ADDRESS, recipient),
        ];

        self.send_get_request(QUOTE_ENDPOINT, &params).await
    }
}
