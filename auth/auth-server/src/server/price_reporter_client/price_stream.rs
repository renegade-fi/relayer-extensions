//! The shared stream of the ETH price

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use atomic_float::AtomicF64;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use renegade_api::websocket::WebsocketMessage;
use serde::Deserialize;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

use crate::http_utils::HttpError;

use super::{error::PriceReporterError, PriceReporterClient};

// -------------
// | Constants |
// -------------

/// The number of milliseconds to wait in between retrying connections
pub const CONN_RETRY_DELAY_MS: u64 = 2_000; // 2 seconds

// ---------
// | Types |
// ---------

/// A type alias for the write end of the websocket connection
type WsWriteStream = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;

/// A type alias for the read end of the websocket connection
type WsReadStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// A message that is sent by the price reporter to the client indicating
/// a price udpate for the given topic
#[derive(Deserialize)]
pub struct PriceMessage {
    /// The topic for which the price update is being sent
    #[allow(dead_code)]
    pub topic: String,
    /// The new price
    pub price: f64,
}

/// The state of the price stream, utilizing atomics for thread-safe access
#[derive(Debug)]
pub struct PriceStreamState {
    /// The latest ETH price in USD
    pub price: AtomicF64,
    /// Whether the websocket is currently connected
    pub is_connected: AtomicBool,
}

impl PriceStreamState {
    /// Create a new price stream state
    pub fn new() -> Self {
        Self { price: AtomicF64::new(0.0), is_connected: AtomicBool::new(false) }
    }

    /// Update the price with a new ETH price
    fn update_price(&self, eth_price: f64) {
        self.price.store(eth_price, Ordering::Relaxed);
    }

    /// Set the connection status
    fn set_connected(&self, connected: bool) {
        self.is_connected.store(connected, Ordering::Relaxed);
    }
}

/// A price stream that manages a WebSocket connection to the price reporter
/// and provides access to the latest ETH price
#[derive(Debug)]
pub struct PriceStream {
    /// The state of the price stream
    state: Arc<PriceStreamState>,
}

// --------------------
// | Public Interface |
// --------------------

impl PriceStream {
    /// Create a new price stream, starting the subscription to the price topic
    pub fn new(ws_url: &str, mint: &str) -> Self {
        let state = Arc::new(PriceStreamState::new());

        let ws_url_clone = ws_url.to_string();
        let mint_clone = mint.to_string();
        let state_clone = state.clone();

        tokio::spawn(async move {
            Self::run_websocket_loop(state_clone, ws_url_clone, mint_clone).await;
        });

        Self { state }
    }

    /// Get the current state of the price stream
    pub fn get_state(&self) -> (f64, bool) {
        let price = self.state.price.load(Ordering::Relaxed);
        let is_connected = self.state.is_connected.load(Ordering::Relaxed);

        (price, is_connected)
    }
}

// -------------------
// | Private Helpers |
// -------------------

impl PriceStream {
    /// The main WebSocket connection loop that handles reconnections
    async fn run_websocket_loop(state: Arc<PriceStreamState>, ws_url: String, mint: String) {
        loop {
            if let Err(e) = Self::stream_price(state.clone(), &ws_url, &mint).await {
                error!("Error streaming prices: {e}");
            }

            state.set_connected(false);
            warn!("Reconnecting to price reporter...");
            tokio::time::sleep(Duration::from_millis(CONN_RETRY_DELAY_MS)).await;
        }
    }

    /// Subscribe to the price topic and handle price updates
    async fn stream_price(
        state: Arc<PriceStreamState>,
        ws_url: &str,
        mint: &str,
    ) -> Result<(), PriceReporterError> {
        let read = connect_and_subscribe(ws_url, mint).await?;
        state.set_connected(true);
        Self::handle_price_updates(read, state).await
    }

    /// Handle price updates from the price reporter, updating the state with
    /// the latest price
    async fn handle_price_updates(
        mut ws_read: WsReadStream,
        state: Arc<PriceStreamState>,
    ) -> Result<(), PriceReporterError> {
        while let Some(res) = ws_read.next().await {
            let msg = res.map_err(PriceReporterError::websocket)?;

            // Attempt to parse price messages from the websocket.
            // Any malformed messages are ignored.
            if let Message::Text(ref text) = msg
                && let Ok(price_message) = serde_json::from_str::<PriceMessage>(text)
            {
                state.update_price(price_message.price);
            } else {
                warn!("Received unknown websocket message: {msg:?}");
            }
        }

        Ok(())
    }
}

// ---------------------
// | Websocket Helpers |
// ---------------------

/// Attempt to connect to the websocket and send a subscription message for the
/// given token mint, returning the read stream
async fn connect_and_subscribe(
    ws_url: &str,
    mint: &str,
) -> Result<WsReadStream, PriceReporterError> {
    let topic = PriceReporterClient::get_price_topic(mint);
    let message = WebsocketMessage::Subscribe { topic };
    let message_ser = Message::text(serde_json::to_string(&message).map_err(HttpError::parsing)?);

    info!("Subscribing to price stream for {mint}...");
    let (mut write, read) = ws_connect(ws_url).await?;
    write.send(message_ser).await.map_err(PriceReporterError::websocket)?;
    Ok(read)
}

/// Build a websocket connection to the given endpoint
async fn ws_connect(ws_url: &str) -> Result<(WsWriteStream, WsReadStream), PriceReporterError> {
    let ws_conn = connect_async(ws_url)
        .await
        .map_err(PriceReporterError::websocket)
        .map(|(conn, _resp)| conn)?;

    let (ws_write, ws_read) = ws_conn.split();
    Ok((ws_write, ws_read))
}
