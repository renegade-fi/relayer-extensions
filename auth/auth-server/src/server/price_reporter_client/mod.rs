//! A client for the price reporter with support for both HTTP and WebSocket
//! connections

use error::PriceReporterError;
use price_stream::PriceStream;
use renegade_common::types::{exchange::Exchange, token::Token};
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

/// A client for the price reporter that supports both HTTP requests
/// and websocket streaming for real-time price updates
#[derive(Debug)]
pub struct PriceReporterClient {
    /// The base URL of the price reporter
    base_url: String,
    /// The price stream for real-time ETH price updates
    eth_price_stream: PriceStream,
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

        let eth = Token::from_ticker(WETH_TICKER);
        Ok(Self { base_url, eth_price_stream: PriceStream::new(ws_url.as_str(), &eth.get_addr()) })
    }

    /// Construct the price topic for a given token
    pub fn get_price_topic(mint: &str) -> String {
        let exchange = Exchange::Binance;
        let quote_mint = Token::from_ticker("USDT").get_addr();
        format!("{}-{}-{}", exchange, mint, quote_mint)
    }

    /// Get the price of a token from the price reporter via HTTP
    pub async fn get_binance_price(&self, mint: &str) -> Result<f64, PriceReporterError> {
        let price_topic = Self::get_price_topic(mint);

        let url = format!("{}{}/{}", self.base_url, PRICE_ROUTE, price_topic);
        let response = send_get_request(&url, DEFAULT_TIMEOUT_SECS).await?;

        let res_text = response.text().await.map_err(HttpError::parsing)?;

        let price: f64 = res_text.parse().map_err(HttpError::parsing)?;

        Ok(price)
    }

    /// Fetch the current price of ETH in USDC.
    ///
    /// We first try reading the state of the ETH price stream,
    /// and fall back to an HTTP request if the stream is not connected.
    pub async fn get_eth_price(&self) -> Result<f64, PriceReporterError> {
        let (price, is_connected) = self.eth_price_stream.get_state();

        if is_connected {
            return Ok(price);
        }

        warn!("Price stream is not connected, fetching ETH price via HTTP");

        // Fall back to HTTP request if the stream is not connected
        let eth = Token::from_ticker(WETH_TICKER);
        self.get_binance_price(&eth.get_addr()).await
    }
}
