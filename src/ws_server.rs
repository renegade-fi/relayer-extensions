//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Duration};

use external_api::websocket::{SubscriptionResponse, WebsocketMessage};
use futures_util::{SinkExt, StreamExt};
use price_reporter::{
    errors::ExchangeConnectionError,
    exchange::{connect_exchange, ExchangeConnection},
    worker::ExchangeConnectionsConfig,
};
use tokio::{
    net::TcpStream,
    sync::{broadcast::channel, RwLock},
    time::Instant,
};
use tokio_stream::StreamMap;
use tokio_tungstenite::accept_async;
use tracing::{debug, error, info, warn};
use tungstenite::Message;
use util::err_str;

use crate::{
    errors::ServerError,
    utils::{
        get_pair_info_topic, get_subscribed_topics, parse_pair_info_from_topic,
        validate_subscription, ClosureSender, PairInfo, PriceMessage, PriceSender, PriceStream,
        PriceStreamMap, SharedPriceStreams, WsWriteStream, CONN_RETRY_DELAY_MS,
        KEEPALIVE_INTERVAL_MS, MAX_CONN_RETRIES, MAX_CONN_RETRY_WINDOW_MS,
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

    /// Initialize a price stream for the given pair info
    pub async fn init_price_stream(
        &mut self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceStream, ServerError> {
        info!("Initializing price stream for {}", get_pair_info_topic(&pair_info));

        // Create a shared channel into which we forward streamed prices
        let (price_tx, price_rx) = channel(32 /* capacity */);

        // Clone the global map of price streams for the task to have access to it
        let global_price_streams = self.clone();

        // Spawn a task responsible for forwarding prices into the broadcast channel &
        // sending keepalive messages to the exchange
        tokio::spawn(async move {
            let res = Self::price_stream_task(
                config,
                pair_info,
                &global_price_streams.price_streams,
                price_tx,
            )
            .await;
            global_price_streams.closure_channel.send(res).unwrap()
        });

        // Return a handle to the broadcast channel stream
        Ok(PriceStream::new(price_rx))
    }

    /// The task responsible for streaming prices from the exchange
    async fn price_stream_task(
        config: ExchangeConnectionsConfig,
        pair_info: PairInfo,
        price_streams: &SharedPriceStreams,
        price_tx: PriceSender,
    ) -> Result<(), ServerError> {
        let mut retry_timestamps = Vec::new();

        // Connect to the pair on the specified exchange
        let mut conn =
            Self::connect_with_retries(&pair_info, &config, &mut retry_timestamps).await?;

        // Add the channel to the map of price streams
        {
            price_streams.write().await.insert(pair_info.clone(), price_tx.clone());
        }

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
                    match price_res.map_err(ServerError::ExchangeConnection) {
                        Ok(price) => {
                            // `send` only errors if there are no more receivers, meaning no more
                            // clientz are subscribed to this price stream. In this case, we remove
                            // the stream from the global map, and complete the task.
                            if price_tx.send(price).is_err() {
                                info!("No more subscribers for {}, closing price stream", get_pair_info_topic(&pair_info));
                                price_streams.write().await.remove(&pair_info);
                                return Ok(());
                            }
                        }
                        Err(e) => {
                            // We failed to stream a price, attempt to
                            // re-establish the connection
                            conn = Self::exhaust_retries(e, &pair_info, &config, &mut retry_timestamps).await?;
                        }
                    }
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
        let (exchange, base, quote) = pair_info;

        // Attempt to connect to the pair on the specified exchange
        match connect_exchange(base, quote, config, *exchange)
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
                Err(e) => e,
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
        warn!("Retrying connection for {}", get_pair_info_topic(pair_info));

        let (exchange, base, quote) = pair_info;

        // Increment the retry count and filter out old requests
        let now = Instant::now();
        let retry_window = Duration::from_millis(MAX_CONN_RETRY_WINDOW_MS);
        retry_timestamps.retain(|ts| now.duration_since(*ts) < retry_window);

        // Add the current timestamp to the set of retries
        retry_timestamps.push(now);

        if retry_timestamps.len() >= MAX_CONN_RETRIES {
            return Err(ServerError::ExchangeConnection(ExchangeConnectionError::MaxRetries(
                *exchange,
            )));
        }

        // Add delay before retrying
        tokio::time::sleep(Duration::from_millis(CONN_RETRY_DELAY_MS)).await;

        // Reconnect
        connect_exchange(base, quote, config, *exchange)
            .await
            .map_err(ServerError::ExchangeConnection)
    }

    /// Fetch a price stream for the given pair info from the global map
    pub async fn get_or_create_price_stream(
        &mut self,
        pair_info: PairInfo,
        config: ExchangeConnectionsConfig,
    ) -> Result<PriceStream, ServerError> {
        let maybe_stream_tx = {
            let price_streams = self.price_streams.read().await;
            price_streams.get(&pair_info).cloned()
        };
        let price_stream = if let Some(stream_tx) = maybe_stream_tx {
            PriceStream::new(stream_tx.subscribe())
        } else {
            self.init_price_stream(pair_info, config).await?
        };

        Ok(price_stream)
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
            Some((pair_info, price_res)) = subscriptions.next() => {
                // The potential error in `price_res` here is a `BroadcastStreamRecvError::Lagged`,
                // meaning the stream lagged receiving price updates. We can safely ignore this.
                if let Ok(price) = price_res {
                    let topic = get_pair_info_topic(&pair_info);
                    let message = PriceMessage { topic, price };
                    let message_ser = serde_json::to_string(&message).map_err(ServerError::Serde)?;
                    write_stream
                        .send(Message::Text(message_ser))
                        .await
                        .map_err(err_str!(ServerError::WebsocketSend))?;
                }
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
                    Ok(res) => serde_json::to_string(&res).map_err(ServerError::Serde)?,
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
    mut global_price_streams: GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
    peer_addr: SocketAddr,
) -> Result<SubscriptionResponse, ServerError> {
    match message {
        WebsocketMessage::Subscribe { topic } => {
            let pair_info = validate_subscription(&topic).await?;

            info!("Subscribing {} to {}", peer_addr, &topic);

            let price_stream =
                global_price_streams.get_or_create_price_stream(pair_info.clone(), config).await?;
            subscriptions.insert(pair_info, price_stream);
        },
        WebsocketMessage::Unsubscribe { topic } => {
            info!("Unsubscribing {} from {}", peer_addr, &topic);
            let pair_info = parse_pair_info_from_topic(&topic)?;
            subscriptions.remove(&pair_info);
        },
    };

    Ok(SubscriptionResponse { subscriptions: get_subscribed_topics(subscriptions) })
}
