//! Defines abstract connection interfaces that can be streamed from

use async_trait::async_trait;
use atomic_float::AtomicF64;
use futures::stream::StreamExt;
use futures_util::{
    Sink, SinkExt, Stream,
    stream::{SplitSink, SplitStream},
};
use renegade_types_core::{Price, Token};
use serde_json::Value;
use std::{
    pin::Pin,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};
use tracing::{error, warn};
use tungstenite::{Error as WsError, http::StatusCode};
use url::Url;

use crate::{
    exchanges::{ExchangeConnectionsConfig, error::ExchangeConnectionError},
    utils::PairInfo,
};

// -------------
// | Constants |
// -------------

/// The message passed when Okx observes a protocol violation
const PROTOCOL_VIOLATION_MSG: &str = "Protocol violation";
/// The message Okx passes in response to a keepalive ping
const PONG_MESSAGE: &str = "pong";
/// The message passed when a ws proxy resets
const CLOUDFLARE_RESET_MESSAGE: &str = "CloudFlare WebSocket proxy restarting";

// ----------------
// | Stream Types |
// ----------------

/// A type alias for a boxed price reader
pub type BoxedPriceReader = Box<dyn Stream<Item = PriceStreamType> + Unpin + Send>;
/// A type alias for a boxed websocket writer
pub type BoxedWsWriter = Box<dyn Sink<Message, Error = WsError> + Unpin + Send>;

/// The type that a price stream should return
pub(crate) type PriceStreamType = Result<Price, ExchangeConnectionError>;

/// A helper struct that represents a stream of midpoint prices that may
/// be initialized at construction
#[derive(Debug)]
pub struct InitializablePriceStream<T: Stream<Item = PriceStreamType> + Unpin> {
    /// The underlying stream
    stream: T,
    /// A buffered stream value, possibly used for initialization
    buffered_value: AtomicF64,
    /// Whether the buffered value has been consumed
    buffered_value_consumed: AtomicBool,
}

impl<T: Stream<Item = PriceStreamType> + Unpin> Stream for InitializablePriceStream<T> {
    type Item = PriceStreamType;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Attempt to consume the buffered value
        if this
            .buffered_value_consumed
            .compare_exchange(
                false, // current
                true,  // new
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            return Poll::Ready(Some(Ok(this.buffered_value.load(Ordering::Relaxed))));
        }

        T::poll_next(Pin::new(&mut this.stream), cx)
    }
}

impl<T: Stream<Item = PriceStreamType> + Unpin> InitializablePriceStream<T> {
    /// Construct a new stream without an initial value
    pub fn new(stream: T) -> Self {
        Self {
            stream,
            buffered_value: AtomicF64::new(0.0),
            buffered_value_consumed: AtomicBool::new(true),
        }
    }

    /// Construct a new stream with an initial value
    pub fn new_with_initial(stream: T, initial_value: Price) -> Self {
        Self {
            stream,
            buffered_value: AtomicF64::new(initial_value),
            buffered_value_consumed: AtomicBool::new(false),
        }
    }
}

// -----------
// | Helpers |
// -----------

/// Build a websocket connection to the given endpoint
pub(crate) async fn ws_connect(
    url: Url,
) -> Result<
    (
        SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
        SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    ),
    ExchangeConnectionError,
> {
    let ws_conn = match connect_async(url.clone()).await {
        Ok((conn, _resp)) => conn,
        Err(e) => {
            error!("Cannot connect to the remote URL: {}", url);
            if let WsError::Http(ref response) = e
                && response.status() == StatusCode::TOO_MANY_REQUESTS
            {
                return Err(ExchangeConnectionError::RateLimited);
            }

            return Err(ExchangeConnectionError::HandshakeFailure(e.to_string()));
        },
    };

    let (ws_sink, ws_stream) = ws_conn.split();
    Ok((ws_sink, ws_stream))
}

/// Send a default ping message on the websocket
pub(super) async fn ws_ping<S: Sink<Message, Error = WsError> + Unpin>(
    ws_sink: &mut S,
) -> Result<(), ExchangeConnectionError> {
    ws_sink
        .send(Message::Ping(vec![]))
        .await
        .map_err(|e| ExchangeConnectionError::SendError(e.to_string()))
}

/// Helper to parse a value from a JSON response
pub(super) fn parse_json_field<T: FromStr>(
    field_name: &str,
    response: &Value,
) -> Result<T, ExchangeConnectionError> {
    match response[field_name].as_str() {
        None => Err(ExchangeConnectionError::InvalidMessage(response.to_string())),
        Some(field_value) => field_value
            .parse()
            .map_err(|_| ExchangeConnectionError::InvalidMessage(response.to_string())),
    }
}

/// Helper to parse a value from a JSON response by index
pub(super) fn parse_json_field_array<T: FromStr>(
    field_index: usize,
    response: &Value,
) -> Result<T, ExchangeConnectionError> {
    match response[field_index].as_str() {
        None => Err(ExchangeConnectionError::InvalidMessage(response.to_string())),
        Some(field_value) => field_value
            .parse()
            .map_err(|_| ExchangeConnectionError::InvalidMessage(response.to_string())),
    }
}

/// Parse an json structure from a websocket message
pub fn parse_json_from_message(
    message: Message,
    pair_info: &PairInfo,
) -> Result<Option<Value>, ExchangeConnectionError> {
    if let Message::Text(message_str) = message {
        // Okx sends some undocumented messages: Empty strings and "Protocol violation"
        // messages
        if message_str == PROTOCOL_VIOLATION_MSG || message_str.is_empty() {
            return Ok(None);
        }

        // Okx sends "pong" messages from our "ping" messages
        if message_str == PONG_MESSAGE {
            return Ok(None);
        }

        // Okx and Kraken send "CloudFlare WebSocket proxy restarting" messages
        if message_str == CLOUDFLARE_RESET_MESSAGE {
            return Ok(None);
        }

        // Parse into a json blob
        serde_json::from_str(&message_str).map_err(|err| {
            ExchangeConnectionError::InvalidMessage(format!("{} for message: {}", err, message_str))
        })
    } else if let Message::Close(close_frame) = message {
        let topic = pair_info.to_topic();
        warn!("Received close message from {topic} websocket");
        if let Some(close_frame) = close_frame {
            let code = close_frame.code;
            let reason = close_frame.reason;
            warn!("Close code: {code}, reason: {reason} for {topic} websocket");
        }
        Ok(None)
    } else {
        Ok(None)
    }
}

// --------------------------
// | Connection Abstraction |
// --------------------------

/// A trait representing a connection to an exchange
#[async_trait]
pub trait ExchangeConnection: Stream<Item = PriceStreamType> + Unpin + Send {
    /// Create a new connection to the exchange on a given asset pair
    async fn connect(
        pair_info: PairInfo,
        config: &ExchangeConnectionsConfig,
    ) -> Result<Self, ExchangeConnectionError>
    where
        Self: Sized;

    /// Send a keepalive signal on the connection if necessary
    async fn send_keepalive(&mut self) -> Result<(), ExchangeConnectionError> {
        Ok(())
    }

    /// Check whether the exchange supports the given pair
    async fn supports_pair(
        base_token: &Token,
        quote_token: &Token,
    ) -> Result<bool, ExchangeConnectionError>
    where
        Self: Sized;
}
