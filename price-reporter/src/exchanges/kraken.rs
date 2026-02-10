//! Defines an abstraction over a Kraken WS connection

use std::{
    collections::HashSet,
    pin::Pin,
    task::{Context, Poll},
};

use async_trait::async_trait;
use futures_util::{SinkExt, Stream, StreamExt};
use lazy_static::lazy_static;
use renegade_types_core::{Exchange, Price, Token};
use renegade_util::err_str;
use serde_json::{Value, json};
use tracing::error;
use tungstenite::Message;
use url::Url;

use crate::{
    exchanges::connection::{
        BoxedPriceReader, BoxedWsWriter, InitializablePriceStream, PriceStreamType,
    },
    utils::PairInfo,
};

use super::{
    ExchangeConnectionsConfig,
    connection::{
        ExchangeConnection, parse_json_field_array, parse_json_from_message, ws_connect, ws_ping,
    },
    error::ExchangeConnectionError,
    util::{exchange_lists_pair_tokens, get_base_exchange_ticker, get_quote_exchange_ticker},
};

// -------------
// | Constants |
// -------------

/// The base URL for the Kraken websocket endpoint
const KRAKEN_WS_BASE_URL: &str = "wss://ws.kraken.com";
/// The base URL for the Kraken REST API
const KRAKEN_REST_BASE_URL: &str = "https://api.kraken.com/0/public";

/// The name of the events field in a Kraken WS message
const KRAKEN_EVENT: &str = "event";
/// The index of the price data in a Kraken WS message
const KRAKEN_PRICE_DATA_INDEX: usize = 1;
/// The index of the bid price in a Kraken WS message's price data
const KRAKEN_BID_PRICE_INDEX: usize = 0;
/// The index of the ask price in a Kraken WS message's price data
const KRAKEN_ASK_PRICE_INDEX: usize = 1;
/// The timestamp of the price report from kraken
const KRAKEN_PRICE_REPORT_TIMESTAMP_INDEX: usize = 2;
/// The name of the error field in a Kraken API response
const KRAKEN_ERROR: &str = "error";

lazy_static! {
    static ref KRAKEN_MSG_IGNORE_LIST: HashSet<String> = {
        let mut set = HashSet::new();

        set.insert(String::from("systemStatus"));
        set.insert(String::from("subscriptionStatus"));
        set.insert(String::from("heartbeat"));
        set
    };
}

// -----------------------------
// | Connection Implementation |
// -----------------------------

/// The message handler for Exchange::Kraken.
pub struct KrakenConnection {
    /// The underlying price stream
    price_stream: BoxedPriceReader,
    /// The underlying write stream of the websocket
    write_stream: BoxedWsWriter,
}

impl KrakenConnection {
    /// Get the URL for the Kraken websocket endpoint
    fn websocket_url() -> Url {
        String::from(KRAKEN_WS_BASE_URL).parse().expect("Failed to parse Kraken websocket URL")
    }

    /// Parse a price report from a Kraken websocket message
    fn midpoint_from_ws_message(
        message: Message,
        pair_info: &PairInfo,
    ) -> Result<Option<Price>, ExchangeConnectionError> {
        // Parse the message to json
        let json_blob = parse_json_from_message(message, pair_info)?;
        if json_blob.is_none() {
            return Ok(None);
        }
        let message_json = json_blob.unwrap();

        // Kraken sends status update messages. Ignore these.
        if KRAKEN_MSG_IGNORE_LIST
            .contains(&message_json[KRAKEN_EVENT].as_str().unwrap_or("").to_string())
        {
            return Ok(None);
        }

        let price_data = &message_json[KRAKEN_PRICE_DATA_INDEX];
        let best_bid: f64 = parse_json_field_array(KRAKEN_BID_PRICE_INDEX, price_data)?;
        let best_offer: f64 = parse_json_field_array(KRAKEN_ASK_PRICE_INDEX, price_data)?;
        let _reported_timestamp_seconds: f32 =
            parse_json_field_array(KRAKEN_PRICE_REPORT_TIMESTAMP_INDEX, price_data)?;

        Ok(Some((best_bid + best_offer) / 2.0))
    }
}

impl Stream for KrakenConnection {
    type Item = PriceStreamType;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.price_stream).poll_next(cx)
    }
}

#[async_trait]
impl ExchangeConnection for KrakenConnection {
    async fn connect(
        pair_info: PairInfo,
        _config: &ExchangeConnectionsConfig,
    ) -> Result<Self, ExchangeConnectionError>
    where
        Self: Sized,
    {
        let base_token = pair_info.base_token();
        let quote_token = pair_info.quote_token();

        // Connect to the websocket
        let url = Self::websocket_url();
        let (mut write, read) = ws_connect(url).await?;

        // Subscribe to the asset pair spread topic
        let base_ticker =
            get_base_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Kraken)?;
        let quote_ticker =
            get_quote_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Kraken)?;

        let pair = format!("{}/{}", base_ticker, quote_ticker);
        let subscribe_str = json!({
            "event": "subscribe",
            "pair": [ pair ],
            "subscription": {
                "name": "spread",
            },
        })
        .to_string();

        write
            .send(Message::Text(subscribe_str))
            .await
            .map_err(|err| ExchangeConnectionError::ConnectionHangup(err.to_string()))?;

        // Map the stream to process midpoint prices
        let mapped_stream = read.filter_map(move |message| {
            let pair_info = pair_info.clone();
            async move {
                match message.map(|message| Self::midpoint_from_ws_message(message, &pair_info)) {
                    // The outer `Result` comes from reading the websocket stream
                    // Processing the stream messages returns a `Result<Option<..>>` which we
                    // flip via `transpose`
                    Ok(val) => val.transpose(),

                    // Error reading from the websocket
                    Err(e) => {
                        error!("Error reading message from Kraken ws: {}", e);
                        Some(Err(ExchangeConnectionError::ConnectionHangup(e.to_string())))
                    },
                }
            }
        });

        // Build a price stream
        let price_stream = InitializablePriceStream::new(Box::pin(mapped_stream));
        Ok(Self { price_stream: Box::new(price_stream), write_stream: Box::new(write) })
    }

    async fn send_keepalive(&mut self) -> Result<(), ExchangeConnectionError> {
        ws_ping(&mut self.write_stream).await
    }

    async fn supports_pair(
        base_token: &Token,
        quote_token: &Token,
    ) -> Result<bool, ExchangeConnectionError> {
        if !exchange_lists_pair_tokens(Exchange::Kraken, base_token, quote_token) {
            return Ok(false);
        }

        let base_ticker = match base_token.get_exchange_ticker(Exchange::Kraken) {
            Some(ticker) => ticker,
            None => return Ok(false),
        };
        let quote_ticker = match quote_token.get_exchange_ticker(Exchange::Kraken) {
            Some(ticker) => ticker,
            None => return Ok(false),
        };

        let pair = format!("{}/{}", base_ticker, quote_ticker);

        // Query the `AssetPairs` endpoint about the pair
        let request_url = format!("{KRAKEN_REST_BASE_URL}/AssetPairs?pair={pair}");

        let response = reqwest::get(request_url)
            .await
            .map_err(err_str!(ExchangeConnectionError::ConnectionHangup))?;

        let res_json: Value =
            response.json().await.map_err(err_str!(ExchangeConnectionError::InvalidMessage))?;

        match &res_json[KRAKEN_ERROR] {
            // No errors => Kraken supports the pair
            Value::Array(errors) => Ok(errors.is_empty()),
            _ => Err(ExchangeConnectionError::InvalidMessage(
                "Invalid response from Kraken".to_string(),
            )),
        }
    }
}
