use renegade_common::types::{exchange::Exchange, token::Token};

use crate::telemetry::sources::http_utils::{send_get_request, HttpError};

/// The route for the price endpoint
pub const PRICE_ROUTE: &str = "/price";
/// Default timeout for requests to the price reporter
const DEFAULT_TIMEOUT_SECS: u64 = 5;

pub struct PriceReporterClient {
    /// The base URL of the price reporter
    base_url: String,
}

impl PriceReporterClient {
    pub fn new(base_url: &str) -> Self {
        Self { base_url: base_url.to_string() }
    }

    pub async fn get_binance_price(&self, mint: &str) -> Result<Option<f64>, HttpError> {
        let exchange = Exchange::Binance;
        let quote_mint = Token::from_ticker("USDT").get_addr();
        let price_topic = format!("{}-{}-{}", exchange, mint, quote_mint);

        let url = format!("{}{}/{}", self.base_url, PRICE_ROUTE, price_topic);
        let response = send_get_request(&url, DEFAULT_TIMEOUT_SECS).await?;

        response
            .text()
            .await
            .map(|text| {
                let price: f64 = text.parse().unwrap();
                Some(price)
            })
            .map_err(|e| HttpError::Network("Failed to parse response".to_string(), e))
    }
}
