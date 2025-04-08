//! A client for the price reporter with support for both HTTP and WebSocket
//! connections

use error::PriceReporterError;
use price_stream::MultiPriceStream;
use renegade_common::types::{
    exchange::Exchange,
    token::{get_all_tokens, Token, USDC_TICKER, USDT_TICKER, USD_TICKER},
};
use reqwest::Url;
use tracing::warn;

use crate::http_utils::{send_get_request, HttpError};

pub mod error;
mod price_stream;

// -------------
// | Constants |
// -------------

/// The price reporter's websocket port
const WS_PORT: u16 = 4000;
/// The route for the price endpoint
pub const PRICE_ROUTE: &str = "/price";
/// Default timeout for requests to the price reporter
const DEFAULT_TIMEOUT_SECS: u64 = 5;

/// The ticker for the WETH token
const WETH_TICKER: &str = "WETH";

/// The tickers of tokens that are excluded from the price stream
const EXCLUDED_TICKERS: [&str; 3] = [USDT_TICKER, USDC_TICKER, USD_TICKER];

/// The error message for an invalid topic
const ERR_INVALID_TOPIC: &str = "Invalid topic format";

/// A client for the price reporter that supports both HTTP requests
/// and websocket streaming for real-time price updates
#[derive(Debug)]
pub struct PriceReporterClient {
    /// The base URL of the price reporter
    base_url: String,
    /// The multi-price stream for real-time token price updates
    multi_price_stream: MultiPriceStream,
}

impl PriceReporterClient {
    /// Create a new PriceReporterClient with the given base URL
    pub fn new(base_url: String) -> Result<Self, PriceReporterError> {
        let mut ws_url: Url = base_url.parse().map_err(HttpError::parsing)?;
        ws_url
            .set_scheme("wss")
            .map_err(|_| PriceReporterError::setup("Error setting websocket scheme"))?;

        ws_url
            .set_port(Some(WS_PORT))
            .map_err(|_| PriceReporterError::setup("Error setting websocket port"))?;

        let mints = get_all_tokens()
            .into_iter()
            .filter(|t| !EXCLUDED_TICKERS.contains(&t.get_ticker().unwrap_or_default().as_str()))
            .map(|t| t.get_addr())
            .collect();

        Ok(Self { base_url, multi_price_stream: MultiPriceStream::new(ws_url.to_string(), mints) })
    }

    /// A convenience method for fetching the current price of ETH in USDC.
    pub async fn get_eth_price(&self) -> Result<f64, PriceReporterError> {
        // Under the hood, the price reporter streams native ETH prices for the WETH
        // token
        let mint = Token::from_ticker(WETH_TICKER).get_addr();
        self.get_price(&mint).await
    }

    /// Fetch the current price of a token from the price reporter.
    ///
    /// We first try reading the state of the price stream,
    /// and fall back to an HTTP request if the stream is not connected.
    pub async fn get_price(&self, mint: &str) -> Result<f64, PriceReporterError> {
        let ws_is_connected = self.multi_price_stream.is_connected();
        if ws_is_connected {
            return Ok(self.multi_price_stream.get_price(mint).await);
        }

        warn!("Price stream is not connected, fetching price via HTTP");
        self.get_price_http(mint).await
    }

    /// Get the price of a token from the price reporter via HTTP
    pub async fn get_price_http(&self, mint: &str) -> Result<f64, PriceReporterError> {
        let price_topic = construct_price_topic(mint);

        let url = format!("{}{}/{}", self.base_url, PRICE_ROUTE, price_topic);
        let response = send_get_request(&url, DEFAULT_TIMEOUT_SECS).await?;

        let res_text = response.text().await.map_err(HttpError::parsing)?;

        let price: f64 = res_text.parse().map_err(HttpError::parsing)?;

        Ok(price)
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Construct the price topic for a given token
pub fn construct_price_topic(mint: &str) -> String {
    let exchange = Exchange::Binance;
    let quote_mint = Token::from_ticker(USDT_TICKER).get_addr();
    format!("{}-{}-{}", exchange, mint, quote_mint)
}

/// Get the base mint from a price topic
pub fn get_base_mint_from_topic(topic: &str) -> Result<String, PriceReporterError> {
    let parts: Vec<&str> = topic.split('-').collect();
    let base_mint = parts.get(1).ok_or(PriceReporterError::parsing(ERR_INVALID_TOPIC))?;
    Ok(base_mint.to_string())
}
