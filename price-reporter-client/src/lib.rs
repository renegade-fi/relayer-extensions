//! A client for the price reporter with support for both HTTP and WebSocket
//! connections

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::unused_async)]
#![feature(let_chains)]

use std::time::Duration;

use bigdecimal::{num_bigint::BigInt, BigDecimal, FromPrimitive};
use error::PriceReporterClientError;
use price_stream::MultiPriceStream;
use renegade_common::types::{
    chain::Chain,
    exchange::Exchange,
    token::{get_all_tokens, Token, USDC_TICKER, USD_TICKER},
};
use reqwest::{Client, Response, Url};
use tracing::warn;

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

// Error messages

/// The tickers of tokens for which we assume a price of `1.0`
const UNIT_PRICE_TICKERS: [&str; 2] = [USDC_TICKER, USD_TICKER];

/// The error message for an invalid topic
const ERR_INVALID_TOPIC: &str = "Invalid topic format";

/// The error message emitted when converting an f64 price to a `BigDecimal`
/// fails
const ERR_PRICE_BIGDECIMAL_CONVERSION: &str = "failed to convert price to BigDecimal";

// ---------
// | Types |
// ---------

/// The configuration options for the price reporter client
#[derive(Debug, Clone, Default)]
pub struct PriceReporterClientConfig {
    /// The base URL of the price reporter
    pub base_url: String,
    /// Whether to disable streaming of prices
    pub disable_price_stream: bool,
    /// Whether to allow the price stream to become stale.
    ///
    /// If `false`, the process will exit when staleness is detected.
    pub allow_stale_price_stream: bool,
}

// ---------------------
// | Client Definition |
// ---------------------

/// A client for the price reporter that supports both HTTP requests
/// and websocket streaming for real-time price updates
#[derive(Debug, Clone)]
pub struct PriceReporterClient {
    /// The base URL of the price reporter
    base_url: String,
    /// The shared HTTP client used for issuing requests to the price reporter
    http_client: Client,
    /// The multi-price stream for real-time token price updates,
    /// if the client was constructed with price streaming enabled
    multi_price_stream: Option<MultiPriceStream>,
}

impl PriceReporterClient {
    /// Create a new PriceReporterClient with the given base URL.
    /// If `exit_on_stale` is true, the process will exit if the price stream
    /// becomes stale.
    pub fn new(config: PriceReporterClientConfig) -> Result<Self, PriceReporterClientError> {
        let PriceReporterClientConfig { base_url, disable_price_stream, allow_stale_price_stream } =
            config;

        // Build a shared HTTP client with a sensible default timeout
        let http_client = Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .map_err(PriceReporterClientError::http)?;

        if disable_price_stream {
            return Ok(Self { base_url, http_client, multi_price_stream: None });
        }

        let mut ws_url: Url = base_url.parse().map_err(PriceReporterClientError::parsing)?;

        if ws_url.scheme() == "https" {
            ws_url
                .set_scheme("wss")
                .map_err(|_| PriceReporterClientError::setup("Error setting websocket scheme"))?;
        } else if ws_url.scheme() == "http" {
            ws_url
                .set_scheme("ws")
                .map_err(|_| PriceReporterClientError::setup("Error setting websocket scheme"))?;
        }

        ws_url
            .set_port(Some(WS_PORT))
            .map_err(|_| PriceReporterClientError::setup("Error setting websocket port"))?;

        let mints = get_all_tokens()
            .into_iter()
            .filter(|t| !UNIT_PRICE_TICKERS.contains(&t.get_ticker().unwrap_or_default().as_str()))
            .map(|t| t.get_addr())
            .collect();

        Ok(Self {
            base_url,
            http_client,
            multi_price_stream: Some(MultiPriceStream::new(
                ws_url.to_string(),
                mints,
                !allow_stale_price_stream,
            )),
        })
    }

    /// A convenience method for fetching the current price of ETH in USDC.
    pub async fn get_eth_price(&self) -> Result<f64, PriceReporterClientError> {
        // Under the hood, the price reporter streams native ETH prices for the WETH
        // token.
        // We assume that whatever chain is set as the default chain in the token
        // mapping has the WETH token - this lets us keep the chain out of the
        // function signature.
        let weth_token = Token::from_ticker(WETH_TICKER);
        let mint = weth_token.get_addr();
        self.get_price(&mint, weth_token.get_chain()).await
    }

    /// Get the decimal-adjusted price of a token, i.e. decimal adjusted units
    /// of USDC per atomic unit of TOKEN
    pub async fn get_price_usd(
        &self,
        mint: &str,
        chain: Chain,
    ) -> Result<BigDecimal, PriceReporterClientError> {
        let price_f64 = self.get_price(mint, chain).await?;
        let price = BigDecimal::from_f64(price_f64)
            .ok_or(PriceReporterClientError::conversion(ERR_PRICE_BIGDECIMAL_CONVERSION))?;

        let decimals = Token::from_addr_on_chain(mint, chain).get_decimals().ok_or_else(|| {
            PriceReporterClientError::custom(format!("Token {mint} has no decimals"))
        })?;

        let adjustment: BigDecimal = BigInt::from(10).pow(decimals as u32).into();
        Ok(price / adjustment)
    }

    /// Get the decimal-adjusted price of a token, i.e. atomic unit of USDC per
    /// atomic unit of TOKEN
    pub async fn get_decimal_adjusted_price(
        &self,
        mint: &str,
        chain: Chain,
    ) -> Result<BigDecimal, PriceReporterClientError> {
        let price_f64 = self.get_price(mint, chain).await?;
        let price = BigDecimal::from_f64(price_f64)
            .ok_or(PriceReporterClientError::conversion(ERR_PRICE_BIGDECIMAL_CONVERSION))?;

        let base_decimals =
            Token::from_addr_on_chain(mint, chain).get_decimals().ok_or_else(|| {
                PriceReporterClientError::custom(format!("Token {mint} has no decimals"))
            })?;
        let usdc_decimals = Token::usdc().get_decimals().expect("USDC has no decimals");

        // Adjust the price
        let adjustment_decimals = usdc_decimals as i32 - base_decimals as i32;
        let adjustment = 10f64.powi(adjustment_decimals);
        let adjustment_bigdecimal = BigDecimal::from_f64(adjustment)
            .ok_or(PriceReporterClientError::conversion(ERR_PRICE_BIGDECIMAL_CONVERSION))?;
        Ok(price * adjustment_bigdecimal)
    }

    /// Fetch the current price of a token from the price reporter.
    ///
    /// We first try reading the state of the price stream,
    /// and fall back to an HTTP request if the stream is not connected.
    pub async fn get_price(
        &self,
        mint: &str,
        chain: Chain,
    ) -> Result<f64, PriceReporterClientError> {
        let mint = mint.to_lowercase();
        let token = Token::from_addr_on_chain(&mint, chain);
        if let Some(ticker) = token.get_ticker()
            && UNIT_PRICE_TICKERS.contains(&ticker.as_str())
        {
            return Ok(1.0);
        }

        let multi_price_stream = self.multi_price_stream.as_ref();
        let ws_is_connected = multi_price_stream.is_some_and(|stream| stream.is_connected());

        if ws_is_connected {
            return multi_price_stream.unwrap().get_price(&mint).await;
        }

        warn!("Price stream is not connected, fetching price via HTTP");
        self.get_price_http(&mint).await
    }

    /// Get the price of a token from the price reporter via HTTP
    pub async fn get_price_http(&self, mint: &str) -> Result<f64, PriceReporterClientError> {
        let price_topic = construct_price_topic(mint);

        let url = format!("{}{}/{}", self.base_url, PRICE_ROUTE, price_topic);
        let response = self.send_get_request(&url).await?;

        let res_text = response.text().await.map_err(PriceReporterClientError::parsing)?;

        let price: f64 = res_text.parse().map_err(PriceReporterClientError::parsing)?;

        Ok(price)
    }

    /// Sends a basic GET request
    async fn send_get_request(&self, url: &str) -> Result<Response, PriceReporterClientError> {
        let response =
            self.http_client.get(url).send().await.map_err(PriceReporterClientError::http)?;

        if !response.status().is_success() {
            let status = response.status();
            let message = response.text().await.map_err(PriceReporterClientError::parsing)?;

            return Err(PriceReporterClientError::http(format!("Status {}: {}", status, message)));
        }

        Ok(response)
    }
}

// ----------------------
// | Non-Member Helpers |
// ----------------------

/// Construct the price topic for a given token
pub fn construct_price_topic(mint: &str) -> String {
    let exchange = Exchange::Renegade;
    format!("{}-{}", exchange, mint)
}

/// Get the base mint from a price topic
pub fn get_base_mint_from_topic(topic: &str) -> Result<String, PriceReporterClientError> {
    let parts: Vec<&str> = topic.split('-').collect();
    let base_mint = parts.get(1).ok_or(PriceReporterClientError::parsing(ERR_INVALID_TOPIC))?;
    Ok(base_mint.to_string())
}
