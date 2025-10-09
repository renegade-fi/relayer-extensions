//! Defines handler logic for a Coinbase websocket connection

use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{SinkExt, Stream, StreamExt};
use jsonwebtoken::{encode, Algorithm, EncodingKey as JwtEncodingKey, Header as JwtHeader};
use rand::Rng;
use renegade_common::types::{exchange::Exchange, price::Price, token::Token};
use renegade_util::{err_str, get_current_time_seconds};
use reqwest::{
    header::{CONTENT_TYPE, USER_AGENT},
    Client,
};
use serde::Serialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tungstenite::Message;
use url::Url;

use crate::{
    exchanges::{
        coinbase::{order_book::CoinbaseOrderBookData, types::CoinbaseOrderBookSnapshotResponse},
        connection::{InitializablePriceStream, PriceStreamType},
        error::ExchangeConnectionError,
        ExchangeConnectionsConfig,
    },
    utils::PairInfo,
};

use super::connection::{
    parse_json_field, parse_json_from_message, ws_connect, ExchangeConnection,
};
use super::util::{
    exchange_lists_pair_tokens, get_base_exchange_ticker, get_quote_exchange_ticker,
};

mod order_book;
mod types;

// -------------
// | Constants |
// -------------

/// The base URL for the Coinbase websocket endpoint
const COINBASE_WS_BASE_URL: &str = "wss://advanced-trade-ws.coinbase.com";
/// The base URL for the Coinbase REST API
const COINBASE_REST_BASE_URL: &str = "https://api.exchange.coinbase.com";
/// The Coinbase developer platform issuer ID
const CDP_ISSUER_ID: &str = "cdp";
/// The User-Agent header for Coinbase requests
const CB_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// The name of the events field in a Coinbase WS message
const COINBASE_EVENTS: &str = "events";
/// The name of the updates field on a coinbase event
const COINBASE_EVENT_UPDATE: &str = "updates";
/// The name of the price level field on a coinbase event
const COINBASE_PRICE_LEVEL: &str = "price_level";
/// The name of the new quantity field on a coinbase event
const COINBASE_NEW_QUANTITY: &str = "new_quantity";
/// The name of the side field on a coinbase event
const COINBASE_SIDE: &str = "side";

/// The bid side field value
const COINBASE_BID: &str = "bid";
/// The offer side field value
const COINBASE_OFFER: &str = "offer";

/// The timeout in seconds for the Coinbase JWT
const COINBASE_JWT_TIMEOUT_SECS: u64 = 60; // 1 minute

/// The min duration on which to refresh the order book from a snapshot
const ORDER_BOOK_REFRESH_INTERVAL_MIN: Duration = Duration::from_secs(30);
/// The max duration on which to refresh the order book from a snapshot
const ORDER_BOOK_REFRESH_INTERVAL_MAX: Duration = Duration::from_secs(60);

/// Build a request URL for the Coinbase order book snapshot
fn order_book_snapshot_url(product_id: &str) -> Url {
    format!("{COINBASE_REST_BASE_URL}/products/{product_id}/book?level=2").parse().unwrap()
}

/// Get a product ID from a base and quote token
fn get_product_id(
    base_token: &Token,
    quote_token: &Token,
) -> Result<String, ExchangeConnectionError> {
    let base_ticker =
        get_base_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Coinbase)?;
    let quote_ticker =
        get_quote_exchange_ticker(base_token.clone(), quote_token.clone(), Exchange::Coinbase)?;

    let product_id = format!("{base_ticker}-{quote_ticker}");
    Ok(product_id)
}

/// The claims for the Coinbase JWT
#[allow(clippy::missing_docs_in_private_items)]
#[derive(Serialize)]
struct CoinbaseJwtClaims {
    #[serde(rename = "sub")]
    subject: String,
    #[serde(rename = "iss")]
    issuer: String,
    #[serde(rename = "nbf")]
    not_before: u64,
    #[serde(rename = "exp")]
    expires: u64,
}

// ----------------------
// | Connection Handler |
// ----------------------

/// The message handler for Exchange::Coinbase.
pub struct CoinbaseConnection {
    /// The underlying stream of prices from the websocket
    price_stream: Box<dyn Stream<Item = PriceStreamType> + Unpin + Send>,
    /// Cancellation token to stop background tasks on drop
    cancel_token: CancellationToken,
}

impl Stream for CoinbaseConnection {
    type Item = PriceStreamType;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        this.price_stream.as_mut().poll_next_unpin(cx)
    }
}

impl CoinbaseConnection {
    // ---------------
    // | API Helpers |
    // ---------------

    /// Get the URL of the Coinbase websocket endpoint
    fn websocket_url() -> Url {
        String::from(COINBASE_WS_BASE_URL).parse().unwrap()
    }

    /// Construct the websocket subscription message with HMAC authentication
    fn construct_subscribe_message(
        product_id: &str,
        config: &ExchangeConnectionsConfig,
    ) -> Result<String, ExchangeConnectionError> {
        let key_name = config.coinbase_key_name.as_ref().expect("coinbase API not configured");
        let key_secret = config.coinbase_key_secret.as_ref().expect("coinbase API not configured");
        let jwt = Self::construct_jwt(key_name, key_secret)?;

        let channel = "level2";
        let subscribe_msg = json!({
            "type": "subscribe",
            "product_ids": [ product_id ],
            "channel": channel,
            "jwt": jwt,
        })
        .to_string();

        Ok(subscribe_msg)
    }

    /// Construct the websocket heartbeat channel subscription with HMAC
    /// authentication
    fn construct_heartbeat_message(
        config: &ExchangeConnectionsConfig,
    ) -> Result<String, ExchangeConnectionError> {
        let key_name = config.coinbase_key_name.as_ref().expect("coinbase API not configured");
        let key_secret = config.coinbase_key_secret.as_ref().expect("coinbase API not configured");
        let jwt = Self::construct_jwt(key_name, key_secret)?;

        let channel = "heartbeat";
        let subscribe_msg = json!({
            "type": "subscribe",
            "channel": channel,
            "jwt": jwt,
        })
        .to_string();

        Ok(subscribe_msg)
    }

    /// Construct a JWT for the Coinbase advanced trade API
    fn construct_jwt(key_name: &str, key_secret: &str) -> Result<String, ExchangeConnectionError> {
        // Parse the key secret as a PEM-encoded EC private key
        let key = JwtEncodingKey::from_ec_pem(key_secret.as_bytes())
            .map_err(err_str!(ExchangeConnectionError::Crypto))?;

        // Build the JWT header and claims
        let mut header = JwtHeader::new(Algorithm::ES256);
        header.kid = Some(key_name.to_string());

        let now = get_current_time_seconds();
        let expires = now + COINBASE_JWT_TIMEOUT_SECS;
        let claims = CoinbaseJwtClaims {
            subject: key_name.to_string(),
            issuer: CDP_ISSUER_ID.to_string(),
            not_before: now,
            expires,
        };

        encode(&header, &claims, &key).map_err(err_str!(ExchangeConnectionError::Crypto))
    }

    /// Fetch an order book snapshot from the HTTP API
    async fn fetch_order_book_snapshot(
        product_id: &str,
    ) -> Result<CoinbaseOrderBookSnapshotResponse, ExchangeConnectionError> {
        let client = Client::new();
        let request_url = order_book_snapshot_url(product_id);
        let response = client
            .get(request_url)
            .header(USER_AGENT, CB_USER_AGENT)
            .send()
            .await
            .map_err(ExchangeConnectionError::custom)?;

        let body = response
            .json::<CoinbaseOrderBookSnapshotResponse>()
            .await
            .map_err(ExchangeConnectionError::custom)?;
        Ok(body)
    }

    // -------------------------
    // | Order Book Management |
    // -------------------------

    /// A loop in which we refresh the order book from a snapshot
    fn start_refresh_order_book_loop(
        product_id: &str,
        order_book: CoinbaseOrderBookData,
        cancel_token: CancellationToken,
    ) {
        let product_id = product_id.to_string();
        tokio::spawn(async move {
            loop {
                if cancel_token.is_cancelled() {
                    info!("Received cancellation signal, stopping order book refresh loop for {product_id}");
                    break;
                }

                if let Err(e) = Self::refresh_order_book(&product_id, &order_book).await {
                    error!("Error refreshing order book: {e}");
                }

                let sleep_time = rand::thread_rng()
                    .gen_range(ORDER_BOOK_REFRESH_INTERVAL_MIN..=ORDER_BOOK_REFRESH_INTERVAL_MAX);

                tokio::time::sleep(sleep_time).await;
            }
        });
    }

    /// Refresh the order book from a snapshot
    async fn refresh_order_book(
        product_id: &str,
        order_book: &CoinbaseOrderBookData,
    ) -> Result<(), ExchangeConnectionError> {
        info!("Refreshing order book for {product_id} from snapshot");
        let snapshot = Self::fetch_order_book_snapshot(product_id).await?;
        order_book.rehydrate(snapshot)?;

        Ok(())
    }

    /// Parse a midpoint price from a websocket message
    fn midpoint_from_ws_message(
        order_book: &CoinbaseOrderBookData,
        message: Message,
        pair_info: &PairInfo,
    ) -> Result<Option<Price>, ExchangeConnectionError> {
        // The json body of the message
        let json = match parse_json_from_message(message, pair_info)? {
            Some(json) => json,
            None => return Ok(None),
        };

        // Extract the list of events and update the order book
        let update_events = if let Some(coinbase_events) = json[COINBASE_EVENTS].as_array()
            && let Some(update_events) = coinbase_events[0][COINBASE_EVENT_UPDATE].as_array()
        {
            update_events
        } else {
            return Ok(None);
        };

        // Make updates to the locally replicated book given the price level updates
        for coinbase_event in update_events {
            let price_level: f64 = parse_json_field(COINBASE_PRICE_LEVEL, coinbase_event)?;
            let new_quantity: f32 = parse_json_field(COINBASE_NEW_QUANTITY, coinbase_event)?;
            let side: String = parse_json_field(COINBASE_SIDE, coinbase_event)?;

            match &side[..] {
                COINBASE_BID => {
                    if new_quantity == 0. {
                        order_book.remove_bid(price_level);
                    } else {
                        order_book.add_bid(price_level);
                    }
                },
                COINBASE_OFFER => {
                    if new_quantity == 0.0 {
                        order_book.remove_offer(price_level);
                    } else {
                        order_book.add_offer(price_level);
                    }
                },
                _ => {
                    return Err(ExchangeConnectionError::InvalidMessage(side.to_string()));
                },
            }
        }

        // Compute the midpoint price
        Ok(order_book.midpoint())
    }
}

impl Drop for CoinbaseConnection {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

#[async_trait]
impl ExchangeConnection for CoinbaseConnection {
    async fn connect(
        pair_info: PairInfo,
        config: &ExchangeConnectionsConfig,
    ) -> Result<Self, ExchangeConnectionError> {
        let base_token = pair_info.base_token();
        let quote_token = pair_info.quote_token();

        // Build the base websocket connection
        let url = Self::websocket_url();
        let (mut writer, read) = ws_connect(url).await?;

        let product_id = get_product_id(&base_token, &quote_token)?;
        let authenticated_subscribe_msg = Self::construct_subscribe_message(&product_id, config)?;
        let authenticated_heartbeat_msg = Self::construct_heartbeat_message(config)?;

        // Setup the topic subscription
        writer
            .send(Message::Text(authenticated_subscribe_msg))
            .await
            .map_err(|err| ExchangeConnectionError::ConnectionHangup(err.to_string()))?;

        // Setup the heartbeat subscription
        writer
            .send(Message::Text(authenticated_heartbeat_msg))
            .await
            .map_err(|err| ExchangeConnectionError::ConnectionHangup(err.to_string()))?;

        // Map the stream of Coinbase messages to one of midpoint prices
        let order_book = CoinbaseOrderBookData::new();
        let order_book_clone = order_book.clone();
        let mapped_stream = read.filter_map(move |message| {
            let pair_info = pair_info.clone();
            let order_book_clone = order_book_clone.clone();
            async move {
                match message {
                    // The outer `Result` comes from reading from the ws stream, the inner `Result`
                    // comes from parsing the message
                    Ok(val) => Self::midpoint_from_ws_message(&order_book_clone, val, &pair_info)
                        .transpose(),

                    Err(e) => {
                        error!("Error reading message from Coinbase websocket: {e}");
                        Some(Err(ExchangeConnectionError::ConnectionHangup(e.to_string())))
                    },
                }
            }
        });

        // Spawn a task to periodically refresh the order book from a snapshot
        let cancel_token = CancellationToken::new();
        Self::start_refresh_order_book_loop(&product_id, order_book, cancel_token.clone());

        // Construct an initialized price stream from the initial price and the mapped
        // stream
        let price_stream = InitializablePriceStream::new(Box::pin(mapped_stream));
        Ok(Self { price_stream: Box::new(price_stream), cancel_token })
    }

    async fn supports_pair(
        base_token: &Token,
        quote_token: &Token,
    ) -> Result<bool, ExchangeConnectionError> {
        if !exchange_lists_pair_tokens(Exchange::Coinbase, base_token, quote_token) {
            return Ok(false);
        }

        let base_ticker = match base_token.get_exchange_ticker(Exchange::Coinbase) {
            Some(ticker) => ticker,
            None => return Ok(false),
        };
        let quote_ticker = match quote_token.get_exchange_ticker(Exchange::Coinbase) {
            Some(ticker) => ticker,
            None => return Ok(false),
        };

        let product_id = format!("{}-{}", base_ticker, quote_ticker);

        // TODO: We sometimes incorrectly report pairs as unsupported due to getting
        // rate limited on the following request

        // Query the `products` endpoint about the pair
        let request_url = format!("{COINBASE_REST_BASE_URL}/products/{product_id}");

        // TODO: Store client on price reporter somewhere to keep connections alive
        let client = Client::new();
        let response = client
            .get(request_url)
            .header(USER_AGENT, CB_USER_AGENT)
            .header(CONTENT_TYPE, "application/json")
            .send()
            .await
            .map_err(err_str!(ExchangeConnectionError::ConnectionHangup))?;

        // A successful response will only be sent if the pair is supported
        Ok(response.status().is_success())
    }
}
