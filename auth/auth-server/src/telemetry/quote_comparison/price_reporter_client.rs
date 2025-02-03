//! A client for the price reporter

use renegade_common::types::{exchange::Exchange, token::Token};

use crate::telemetry::sources::http_utils::{send_get_request, HttpError};

/// The route for the price endpoint
pub const PRICE_ROUTE: &str = "/price";
/// Default timeout for requests to the price reporter
const DEFAULT_TIMEOUT_SECS: u64 = 5;

/// A client for the price reporter
#[derive(Clone)]
pub struct PriceReporterClient {
    /// The base URL of the price reporter
    base_url: String,
}

impl PriceReporterClient {
    /// Create a new PriceReporterClient
    pub fn new(base_url: &str) -> Self {
        Self { base_url: base_url.to_string() }
    }

    /// Get the price of a token from the price reporter
    pub async fn get_binance_price(&self, mint: &str) -> Result<f64, HttpError> {
        let exchange = Exchange::Binance;
        let quote_mint = Token::from_ticker("USDT").get_addr();
        let price_topic = format!("{}-{}-{}", exchange, mint, quote_mint);

        let url = format!("{}{}/{}", self.base_url, PRICE_ROUTE, price_topic);
        let response = send_get_request(&url, DEFAULT_TIMEOUT_SECS).await?;

        let res_text = response
            .text()
            .await
            .map_err(|e| HttpError::Network("Failed to get response text".to_string(), e))?;

        let price: f64 = res_text.parse().map_err(HttpError::parsing)?;

        Ok(price)
    }

    /// Fetch the current price of ETH in USDC.
    ///
    /// Under the hood, the price reporter fetches the ETH price instead of
    /// WETH.
    pub async fn get_eth_price(&self) -> Result<f64, HttpError> {
        let eth = Token::from_ticker("WETH");
        self.get_binance_price(&eth.get_addr()).await
    }
}
