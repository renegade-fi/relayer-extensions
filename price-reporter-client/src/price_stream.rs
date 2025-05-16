//! The shared stream of the ETH price

use std::{
    collections::HashMap,
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
use tokio::{net::TcpStream, sync::RwLock};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use tracing::{error, info, warn};

use super::{construct_price_topic, error::PriceReporterClientError, get_base_mint_from_topic};

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

/// A type alias for a synchronized map from token mints to their latest prices
type SyncPricesMap = RwLock<HashMap<String, AtomicF64>>;

/// A message that is sent by the price reporter to the client indicating
/// a price udpate for the given topic
#[derive(Deserialize)]
pub struct PriceMessage {
    /// The topic for which the price update is being sent
    pub topic: String,
    /// The new price
    pub price: f64,
}

/// The thread-safe state of the multi-price stream
#[derive(Debug)]
pub struct MultiPriceStreamState {
    /// The latest prices for the tokens managed by the price stream
    pub prices: SyncPricesMap,
    /// Whether the websocket is currently connected
    pub is_connected: AtomicBool,
}

impl MultiPriceStreamState {
    /// Create a new multi-price stream state
    pub fn new() -> Self {
        Self { prices: SyncPricesMap::new(HashMap::new()), is_connected: AtomicBool::new(false) }
    }

    /// Update the price of a token
    async fn update_price(&self, mint: String, price: f64) {
        self.prices
            .write()
            .await
            .entry(mint)
            .or_insert(AtomicF64::new(price))
            .store(price, Ordering::Relaxed);
    }

    /// Set the connection status
    fn set_connected(&self, connected: bool) {
        self.is_connected.store(connected, Ordering::Relaxed);
    }
}

/// A multi-price stream that manages a WebSocket connection to the price
/// reporter and provides access to the latest prices of the desired tokens
#[derive(Debug)]
pub struct MultiPriceStream {
    /// The inner state of the multi-price stream, made shareable via an `Arc`
    /// so that it can be updated by the websocket thread
    inner: Arc<MultiPriceStreamState>,
}

// --------------------
// | Public Interface |
// --------------------

impl MultiPriceStream {
    /// Create a new multi-price stream, starting the subscription to the price
    /// topics
    pub fn new(ws_url: String, mints: Vec<String>) -> Self {
        let inner = Arc::new(MultiPriceStreamState::new());
        let inner_clone = inner.clone();

        tokio::spawn(async move {
            Self::run_websocket_loop(inner_clone, ws_url, mints).await;
        });

        Self { inner }
    }

    /// Get the current state of the price stream
    pub async fn get_price(&self, mint: &str) -> f64 {
        self.inner.prices.read().await.get(mint).map_or(0.0, |price| price.load(Ordering::Relaxed))
    }

    /// Get the connection status of the price stream
    pub fn is_connected(&self) -> bool {
        self.inner.is_connected.load(Ordering::Relaxed)
    }
}

// -------------------
// | Private Helpers |
// -------------------

impl MultiPriceStream {
    /// The main WebSocket connection loop that handles reconnections
    async fn run_websocket_loop(
        state: Arc<MultiPriceStreamState>,
        ws_url: String,
        mints: Vec<String>,
    ) {
        loop {
            if let Err(e) = Self::stream_prices(state.clone(), &ws_url, &mints).await {
                error!("Error streaming prices: {e}");
            }

            state.set_connected(false);
            warn!("Reconnecting to price reporter...");
            tokio::time::sleep(Duration::from_millis(CONN_RETRY_DELAY_MS)).await;
        }
    }

    /// Subscribe to the price topics and handle price updates
    async fn stream_prices(
        state: Arc<MultiPriceStreamState>,
        ws_url: &str,
        mints: &[String],
    ) -> Result<(), PriceReporterClientError> {
        let read = connect_and_subscribe(ws_url, mints).await?;
        state.set_connected(true);
        Self::handle_price_updates(read, state).await
    }

    /// Handle price updates from the price reporter, updating the state with
    /// the latest prices
    async fn handle_price_updates(
        mut ws_read: WsReadStream,
        state: Arc<MultiPriceStreamState>,
    ) -> Result<(), PriceReporterClientError> {
        while let Some(res) = ws_read.next().await {
            let msg = res.map_err(PriceReporterClientError::websocket)?;

            // Attempt to parse price messages from the websocket.
            // Any malformed messages are ignored.
            if let Message::Text(ref text) = msg
                && let Ok(price_message) = serde_json::from_str::<PriceMessage>(text)
            {
                let mint = get_base_mint_from_topic(&price_message.topic)?;
                state.update_price(mint, price_message.price).await;
            }
        }

        Ok(())
    }
}

// ---------------------
// | Websocket Helpers |
// ---------------------

/// Attempt to connect to the websocket and send a subscription message for each
/// of the given token mints, returning the read stream
async fn connect_and_subscribe(
    ws_url: &str,
    mints: &[String],
) -> Result<WsReadStream, PriceReporterClientError> {
    let (mut write, read) = ws_connect(ws_url).await?;

    for mint in mints {
        let topic = construct_price_topic(mint);
        let message = WebsocketMessage::Subscribe { topic };
        let message_ser = Message::text(
            serde_json::to_string(&message).map_err(PriceReporterClientError::parsing)?,
        );

        info!("Subscribing to price stream for {mint}...");
        write.send(message_ser).await.map_err(PriceReporterClientError::websocket)?;
    }

    Ok(read)
}

/// Build a websocket connection to the given endpoint
async fn ws_connect(
    ws_url: &str,
) -> Result<(WsWriteStream, WsReadStream), PriceReporterClientError> {
    let ws_conn = connect_async(ws_url)
        .await
        .map_err(PriceReporterClientError::websocket)
        .map(|(conn, _resp)| conn)?;

    let (ws_write, ws_read) = ws_conn.split();
    Ok((ws_write, ws_read))
}
