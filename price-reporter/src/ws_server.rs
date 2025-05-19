//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use renegade_api::websocket::{SubscriptionResponse, WebsocketMessage};
use renegade_common::types::{exchange::Exchange, Price};
use renegade_price_reporter::{
    errors::ExchangeConnectionError,
    exchange::{connect_exchange, ExchangeConnection},
    worker::ExchangeConnectionsConfig,
};
use renegade_util::err_str;
use tokio::{net::TcpStream, sync::watch::channel, sync::RwLock, time::Instant};
use tokio_stream::StreamMap;
use tokio_tungstenite::accept_async;
use tracing::{debug, error, info, warn};
use tungstenite::Message;

use crate::{
    errors::ServerError,
    utils::{
        get_price_topic_str, get_subscribed_topics, ClosureSender, PairInfo, PriceMessage,
        PriceReceiver, PriceSender, PriceStream, PriceStreamMap, SharedPriceStreams, WsWriteStream,
        CONN_RETRY_DELAY_MS, KEEPALIVE_INTERVAL_MS, MAX_CONN_RETRIES, MAX_CONN_RETRY_WINDOW_MS,
    },
};

// ----------------------------
// | GLOBAL PRICE STREAMS MAP |
// ----------------------------

/// A map of price streams from exchanges maintained by the server,
/// shared across all connections
#[derive(Clone)]
pub struct GlobalPriceStreams {
    /// A thread-safe map of price streams, indexed by the (source, base, quote)
    /// tuple
    pub price_streams: SharedPriceStreams,
    /// A channel to send closure signals from the price stream tasks
    pub closure_channel: ClosureSender,
}

impl GlobalPriceStreams {
    /// Instantiate a new global price streams map
    pub fn new(closure_channel: ClosureSender) -> Self {
        Self { price_streams: Arc::new(RwLock::new(HashMap::new())), closure_channel }
    }

    /// Add a price stream to the global map
    pub async fn add_price_stream(&self, pair_info: PairInfo, price_rx: PriceReceiver) {
        self.price_streams.write().await.insert(pair_info, price_rx);
    }

    /// Remove a price stream from the global map
    pub async fn remove_price_stream(&self, pair_info: PairInfo) {
        self.price_streams.write().await.remove(&pair_info);
    }

    /// Initialize a price stream for the given pair info
    pub async fn init_price_stream(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        pair_info.validate_subscription().await?;

        info!("Initializing price stream for {}", pair_info.to_topic());

        // Create a shared channel into which we forward streamed prices
        let (price_tx, price_rx) = channel(Price::default());
        self.add_price_stream(pair_info.clone(), price_rx.clone()).await;

        // Spawn a task responsible for forwarding prices into the broadcast channel &
        // sending keepalive messages to the exchange
        let global_price_streams = self.clone();
        tokio::spawn(async move {
            let res = Self::price_stream_task(config, pair_info.clone(), price_tx).await;
            global_price_streams.remove_price_stream(pair_info).await;
            global_price_streams.closure_channel.send(res).unwrap()
        });

        // Return a handle to the broadcast channel stream
        Ok(price_rx)
    }

    /// The task responsible for streaming prices from the exchange
    async fn price_stream_task(
        config: ExchangeConnectionsConfig,
        pair_info: PairInfo,
        price_tx: PriceSender,
    ) -> Result<(), ServerError> {
        let mut retry_timestamps = Vec::new();

        // Connect to the pair on the specified exchange
        let mut conn =
            Self::connect_with_retries(&pair_info, &config, &mut retry_timestamps).await?;

        loop {
            match Self::manage_connection(&mut conn, &price_tx).await {
                Ok(()) => {},
                Err(e) => {
                    conn = Self::exhaust_retries(e, &pair_info, &config, &mut retry_timestamps)
                        .await?;
                },
            }
        }
    }

    /// Manages an exchange connection, sending keepalive messages and
    /// forwarding prices to the price receiver
    async fn manage_connection(
        conn: &mut Box<dyn ExchangeConnection>,
        price_tx: &PriceSender,
    ) -> Result<(), ServerError> {
        let delay = tokio::time::sleep(Duration::from_millis(KEEPALIVE_INTERVAL_MS));
        tokio::pin!(delay);

        loop {
            tokio::select! {
                // Send a keepalive message to the exchange
                _ = &mut delay => {
                    conn.send_keepalive().await.map_err(ServerError::ExchangeConnection)?;
                    delay.as_mut().reset(Instant::now() + Duration::from_millis(KEEPALIVE_INTERVAL_MS));
                }

                // Forward the next price into the broadcast channel
                Some(price_res) = conn.next() => {
                    let price = price_res.map_err(ServerError::ExchangeConnection)?;
                    let _ = price_tx.send(price);
                }
            }
        }
    }

    /// Initialize an exchange connection, retrying if necessary
    async fn connect_with_retries(
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        let (exchange, base, quote) =
            (pair_info.exchange, pair_info.base_token(), pair_info.quote_token());

        // Attempt to connect to the pair on the specified exchange
        match connect_exchange(&base, &quote, config, exchange)
            .await
            .map_err(ServerError::ExchangeConnection)
        {
            Ok(conn) => Ok(conn),
            Err(e) => Self::exhaust_retries(e, pair_info, config, retry_timestamps).await,
        }
    }

    /// Attempt to re-establish an erroring exchange connection, exhausting
    /// retries if necessary
    async fn exhaust_retries(
        mut prev_err: ServerError,
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        let exchange = pair_info.exchange;
        loop {
            prev_err = match Self::retry_connection(pair_info, config, retry_timestamps).await {
                Ok(conn) => return Ok(conn),
                Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                    exchange,
                ))) => {
                    // Return the original error if we've exhausted retries
                    error!("Exhausted retries for {}", exchange);
                    return Err(prev_err);
                },
                Err(e) => {
                    warn!("Failed to reconnect to {exchange}: {e}");
                    e
                },
            };
        }
    }

    /// Retries an exchange connection after it has failed.
    ///
    /// Mirrors https://github.com/renegade-fi/renegade/blob/main/workers/price-reporter/src/reporter.rs#L470
    async fn retry_connection(
        pair_info: &PairInfo,
        config: &ExchangeConnectionsConfig,
        retry_timestamps: &mut Vec<Instant>,
    ) -> Result<Box<dyn ExchangeConnection>, ServerError> {
        warn!("Retrying connection for {}", pair_info.to_topic());

        let (exchange, base, quote) =
            (pair_info.exchange, pair_info.base_token(), pair_info.quote_token());

        // Increment the retry count and filter out old requests
        let now = Instant::now();
        let retry_window = Duration::from_millis(MAX_CONN_RETRY_WINDOW_MS);
        retry_timestamps.retain(|ts| now.duration_since(*ts) < retry_window);

        // Add the current timestamp to the set of retries
        retry_timestamps.push(now);

        if retry_timestamps.len() >= MAX_CONN_RETRIES {
            return Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                exchange,
            )));
        }

        // Add delay before retrying
        tokio::time::sleep(Duration::from_millis(CONN_RETRY_DELAY_MS)).await;

        // Reconnect
        connect_exchange(&base, &quote, config, exchange)
            .await
            .map_err(ServerError::ExchangeConnection)
    }

    /// Returns a tuple of (canonicalized pair info, requires quote conversion),
    /// if needed
    fn normalize_pair_info(&self, pair_info: PairInfo) -> Result<(PairInfo, bool), ServerError> {
        if pair_info.exchange != Exchange::Renegade {
            return Ok((pair_info, false));
        }

        let base_mint = pair_info.base_token().get_addr();
        let new_pair_info = PairInfo::new_canonical_exchange(&base_mint)?;
        let requires_conversion = pair_info.requires_quote_conversion()?;
        Ok((new_pair_info, requires_conversion))
    }

    /// Fetch a price stream for the given pair info from the global map
    pub async fn get_or_create_price_stream(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceStream, ServerError> {
        let (normalized_pair_info, requires_conversion) =
            self.normalize_pair_info(pair_info.clone())?;

        let price_rx =
            self.get_or_create_price_receiver(normalized_pair_info.clone(), config.clone()).await?;
        let stream = if requires_conversion {
            let conversion_rx = self.quote_conversion_stream(normalized_pair_info, config).await?;
            PriceStream::new_with_conversion(price_rx.into(), conversion_rx.into())
        } else {
            PriceStream::new(price_rx.into())
        };

        Ok(stream)
    }

    /// Get a quote conversion stream for a given exchange
    ///
    /// Currently we only need to convert USDT -> USDC for `Renegade` prices, so
    /// this method does not configure the conversion tokens.
    async fn quote_conversion_stream(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        let conversion_pair = pair_info.get_conversion_pair();
        let conversion_rx = self.get_or_create_price_receiver(conversion_pair, config).await?;
        Ok(conversion_rx)
    }

    /// Get a price receiver for the given pair or create a new stream
    async fn get_or_create_price_receiver(
        &self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceReceiver, ServerError> {
        let maybe_stream_rx = {
            let price_streams = self.price_streams.read().await;
            price_streams.get(&pair_info).cloned()
        };

        let recv = match maybe_stream_rx {
            Some(stream_rx) => stream_rx,
            None => self.init_price_stream(pair_info, config).await?,
        };

        Ok(recv)
    }
}

// ----------
// | SERVER |
// ----------

/// Handles an incoming websocket connection,
/// establishing a listener loop for subscription requests
pub async fn handle_connection(
    stream: TcpStream,
    global_price_streams: GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
) -> Result<(), ServerError> {
    let peer_addr = stream.peer_addr().map_err(ServerError::GetPeerAddr)?;

    debug!("Accepting websocket connection from: {}", peer_addr);

    let websocket_stream =
        accept_async(stream).await.map_err(err_str!(ServerError::WebsocketConnection))?;
    let (mut write_stream, mut read_stream) = websocket_stream.split();

    let mut subscriptions = StreamMap::new();

    loop {
        tokio::select! {
            // Send the next price to the client
            Some((topic, price)) = subscriptions.next() => {
                // The potential error in `price_res` here is a `BroadcastStreamRecvError::Lagged`,
                // meaning the stream lagged receiving price updates. We can safely ignore this.
                let topic = get_price_topic_str(&topic);
                let message = PriceMessage { topic, price };
                let message_ser = serde_json::to_string(&message).map_err(err_str!(ServerError::Serde))?;
                write_stream
                    .send(Message::Text(message_ser))
                    .await
                    .map_err(err_str!(ServerError::WebsocketSend))?;
            }

            // Handle incoming websocket messages
            message = read_stream.next() => {
                match message {
                    Some(msg) => {
                        let msg_inner = msg.map_err(err_str!(ServerError::WebsocketReceive))?;

                        match msg_inner {
                            Message::Close(_) => break,
                            _ => {
                                handle_ws_message(
                                    msg_inner,
                                    &mut subscriptions,
                                    &mut write_stream,
                                    global_price_streams.clone(),
                                    config.clone(),
                                    peer_addr,
                                ).await?;
                            }
                        }
                    }

                    // None is returned when the connection is closed or a critical error
                    // occurred. In either case the server side may hang up
                    None => break
                }
            }
        }
    }

    debug!("Closing websocket connection from: {}", peer_addr);

    Ok(())
}

/// Handles an incoming websocket message
async fn handle_ws_message(
    message: Message,
    subscriptions: &mut PriceStreamMap,
    write_stream: &mut WsWriteStream,
    global_price_streams: GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
    peer_addr: SocketAddr,
) -> Result<(), ServerError> {
    if let Message::Text(msg_text) = message {
        let msg_deser: Result<WebsocketMessage, _> = serde_json::from_str(&msg_text);
        let resp = match msg_deser {
            // Valid message body
            Ok(msg) => {
                let response = match handle_subscription_message(
                    msg,
                    subscriptions,
                    global_price_streams,
                    config,
                    peer_addr,
                )
                .await
                {
                    Ok(res) => serde_json::to_string(&res).map_err(err_str!(ServerError::Serde))?,
                    Err(e) => e.to_string(),
                };

                Message::Text(response)
            },

            // Respond with an error if deserialization fails
            Err(e) => Message::Text(format!("Invalid request: {}", e)),
        };

        // Write out the response over the websocket
        write_stream.send(resp).await.map_err(err_str!(ServerError::WebsocketSend))?;
    }

    Ok(())
}

/// Handles an incoming un/subscribe message
async fn handle_subscription_message(
    message: WebsocketMessage,
    subscriptions: &mut PriceStreamMap,
    global_price_streams: GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
    peer_addr: SocketAddr,
) -> Result<SubscriptionResponse, ServerError> {
    match message {
        WebsocketMessage::Subscribe { topic } => {
            info!("Subscribing {} to {}", peer_addr, &topic);
            let pair_info = PairInfo::from_topic(&topic)?;
            let stream = global_price_streams
                .get_or_create_price_stream(pair_info.clone(), config.clone())
                .await?;

            subscriptions.insert(pair_info.into(), stream);
        },
        WebsocketMessage::Unsubscribe { topic } => {
            info!("Unsubscribing {} from {}", peer_addr, &topic);
            let pair_info = PairInfo::from_topic(&topic)?;
            subscriptions.remove(&pair_info.into());
        },
    };

    Ok(SubscriptionResponse { subscriptions: get_subscribed_topics(subscriptions) })
}
