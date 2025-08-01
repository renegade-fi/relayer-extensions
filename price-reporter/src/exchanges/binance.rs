//! Defines the logic for connecting to a binance exchange

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use async_trait::async_trait;
use futures_util::{Sink, Stream, StreamExt};
use renegade_common::types::{
    exchange::{Exchange, PriceReport},
    price::Price,
    token::Token,
};
use renegade_util::{err_str, get_current_time_millis};
use serde_json::Value;
use tracing::error;
use tungstenite::{Error as WsError, Message};
use url::Url;

use crate::exchanges::{
    connection::{InitializablePriceStream, PriceStreamType},
    error::ExchangeConnectionError,
    util::{exchange_lists_pair_tokens, get_base_exchange_ticker, get_quote_exchange_ticker},
    ExchangeConnectionsConfig,
};

use super::connection::{
    parse_json_field, parse_json_from_message, ws_connect, ws_ping, ExchangeConnection,
};

// -------------
// | Constants |
// -------------

/// The base URL for the Binance websocket endpoint
const BINANCE_WS_BASE_URL: &str = "wss://stream.binance.com:443/ws";
/// The base URL for the Binance REST API
const BINANCE_REST_BASE_URL: &str = "https://api.binance.com/api/v3";

/// The name of the midpoint bid price on an HTTP response
const BINANCE_BID_PRICE: &str = "bidPrice";
/// The name of the midpoint offer price on an HTTP response
const BINANCE_OFFER_PRICE: &str = "askPrice";

/// The name of the best bid field in a websocket message
const BINANCE_BID_PRICE_WS: &str = "b";
/// The name of the best offer field in a websocket message
const BINANCE_OFFER_PRICE_WS: &str = "a";

// --------------
// | Connection |
// --------------

/// The connection handle for Binance price data
pub struct BinanceConnection {
    /// The underlying price stream
    ///
    /// TODO: Unbox this if performance becomes a concern
    price_stream: Box<dyn Stream<Item = PriceStreamType> + Unpin + Send>,
    /// The underlying write stream of the websocket
    write_stream: Box<dyn Sink<Message, Error = WsError> + Unpin + Send>,
}

impl BinanceConnection {
    /// Construct the websocket url for the given asset pair
    fn websocket_url(
        base_token: Token,
        quote_token: Token,
    ) -> Result<Url, ExchangeConnectionError> {
        let base_ticker =
            get_base_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Binance)?;

        let quote_ticker = get_quote_exchange_ticker(base_token, quote_token, Exchange::Binance)?;

        let url = Url::parse(&format!(
            "{BINANCE_WS_BASE_URL}/{}{}@bookTicker",
            base_ticker.to_lowercase(),
            quote_ticker.to_lowercase()
        ))
        .expect("url parse should not fail on valid format string");

        Ok(url)
    }

    /// Fetch a one-off price report by polling the Binance REST API
    async fn fetch_price_report(
        base_token: Token,
        quote_token: Token,
    ) -> Result<PriceReport, ExchangeConnectionError> {
        // Make the request
        let base_ticker =
            get_base_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Binance)?;
        let quote_ticker =
            get_quote_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Binance)?;
        let request_url = format!(
            "{BINANCE_REST_BASE_URL}/ticker/bookTicker?symbol={}{}",
            base_ticker, quote_ticker
        );

        let message_resp = reqwest::get(request_url)
            .await
            .map_err(|err| ExchangeConnectionError::ConnectionHangup(err.to_string()))?;

        // Parse the prices and return a price report
        let message_json: Value = message_resp
            .json()
            .await
            .map_err(|err| ExchangeConnectionError::ConnectionHangup(err.to_string()))?;

        let best_bid: f64 = parse_json_field(BINANCE_BID_PRICE, &message_json)?;
        let best_offer: f64 = parse_json_field(BINANCE_OFFER_PRICE, &message_json)?;
        let midpoint_price = (best_bid + best_offer) / 2.0;

        Ok(PriceReport {
            base_token,
            quote_token,
            price: midpoint_price,
            local_timestamp: get_current_time_millis(),
        })
    }

    /// Parse a price report from an incoming message
    fn midpoint_from_ws_message(
        message: Message,
    ) -> Result<Option<Price>, ExchangeConnectionError> {
        // Deserialize the message into a JSON object
        if let Some(json_blob) = parse_json_from_message(message)? {
            // Raw numbers are ignored
            if let Value::Number(_) = json_blob {
                return Ok(None);
            }

            let best_bid: f64 = parse_json_field(BINANCE_BID_PRICE_WS, &json_blob)?;
            let best_offer: f64 = parse_json_field(BINANCE_OFFER_PRICE_WS, &json_blob)?;

            Ok(Some((best_bid + best_offer) / 2.0))
        } else {
            Ok(None)
        }
    }
}

impl Stream for BinanceConnection {
    type Item = PriceStreamType;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.price_stream.as_mut().poll_next_unpin(cx)
    }
}

#[async_trait]
impl ExchangeConnection for BinanceConnection {
    async fn connect(
        base_token: Token,
        quote_token: Token,
        _config: &ExchangeConnectionsConfig,
    ) -> Result<Self, ExchangeConnectionError>
    where
        Self: Sized,
    {
        // Fetch an inital price report to setup the stream
        let initial_price_report =
            Self::fetch_price_report(base_token.clone(), quote_token.clone()).await?;

        // Connect to the websocket
        let url = Self::websocket_url(base_token, quote_token)?;
        let (write, read) = ws_connect(url).await?;

        // Map the stream to process midpoint prices
        let mapped_stream = read.filter_map(|message| async {
            match message.map(Self::midpoint_from_ws_message) {
                // The result is for reading a message from the websocket, the inner result is for
                // processing that message and returns an option. Flip the order of the option and
                // the result in the processed message
                Ok(mapped_stream) => mapped_stream.transpose(),
                // Error on the incoming (filtered) stream
                Err(e) => {
                    error!("Error reading message from Binance ws: {}", e);
                    Some(Err(ExchangeConnectionError::ConnectionHangup(e.to_string())))
                },
            }
        });

        // Construct an initialized price stream from the initial price and the mapped
        // stream
        let price_stream = InitializablePriceStream::new_with_initial(
            Box::pin(mapped_stream),
            initial_price_report.price,
        );

        Ok(Self { price_stream: Box::new(price_stream), write_stream: Box::new(write) })
    }

    async fn send_keepalive(&mut self) -> Result<(), ExchangeConnectionError> {
        ws_ping(&mut self.write_stream).await
    }

    async fn supports_pair(
        base_token: &Token,
        quote_token: &Token,
    ) -> Result<bool, ExchangeConnectionError> {
        if !exchange_lists_pair_tokens(Exchange::Binance, base_token, quote_token) {
            return Ok(false);
        }

        let base_ticker = match base_token.get_exchange_ticker(Exchange::Binance) {
            Some(ticker) => ticker,
            None => return Ok(false),
        };
        let quote_ticker = match quote_token.get_exchange_ticker(Exchange::Binance) {
            Some(ticker) => ticker,
            None => return Ok(false),
        };

        let symbol = format!("{}{}", base_ticker, quote_ticker);

        // Query the `exchangeInfo` endpoint about the pair
        let request_url = format!("{BINANCE_REST_BASE_URL}/exchangeInfo?symbol={symbol}");

        let response = reqwest::get(request_url)
            .await
            .map_err(err_str!(ExchangeConnectionError::ConnectionHangup))?;

        // A successful response will only be sent if the pair is supported
        Ok(response.status().is_success())
    }
}
