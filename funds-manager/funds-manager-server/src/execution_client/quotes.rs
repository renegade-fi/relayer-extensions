//! Client methods for fetching quotes and prices from the execution venue

use funds_manager_api::quoters::ExecutionQuote;
use serde::Deserialize;

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
