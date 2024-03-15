//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::{collections::HashMap, sync::Arc, time::Duration};

use external_api::websocket::{SubscriptionResponse, WebsocketMessage};
use futures_util::{SinkExt, StreamExt};
use price_reporter::{
    exchange::{connect_exchange, ExchangeConnection},
    reporter::KEEPALIVE_INTERVAL_MS,
    worker::ExchangeConnectionsConfig,
};
use tokio::{
    net::TcpStream,
    sync::{broadcast::channel, RwLock},
    time::Instant,
};
use tokio_stream::StreamMap;
use tokio_tungstenite::accept_async;
use tungstenite::Message;
use util::err_str;

use crate::{
    errors::ServerError,
    utils::{
        get_pair_info_topic, get_subscribed_topics, parse_pair_info_from_topic, ClosureSender, PairInfo, PriceMessage, PriceSender, PriceStream, PriceStreamMap, SharedPriceStreams, WsWriteStream
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
        config: &ExchangeConnectionsConfig,
    ) -> Result<PriceStream, ServerError> {
        let (exchange, base, quote) = pair_info.clone();

        // Connect to the pair on the specified exchange
        let conn = connect_exchange(&base, &quote, config, exchange)
            .await
            .map_err(ServerError::ExchangeConnection)?;

        // Create a shared channel into which we forward streamed prices
        let (price_tx, price_rx) = channel(32 /* capacity */);

        // Clone the global map of price streams for the task to have access to it
        let global_price_streams = self.clone();

        // Spawn a task responsible for forwarding prices into the broadcast channel &
        // sending keepalive messages to the exchange
        tokio::spawn(async move {
            let res =
                Self::price_stream_task(pair_info, &global_price_streams.price_streams, conn, price_tx).await;
            global_price_streams.closure_channel.send(res).unwrap()
        });

        // Return a handle to the broadcast channel stream
        Ok(PriceStream::new(price_rx))
    }

    /// The task responsible for streaming prices from the exchange
    async fn price_stream_task(
        pair_info: PairInfo,
        price_streams: &SharedPriceStreams,
        mut conn: Box<dyn ExchangeConnection>,
        price_tx: PriceSender,
    ) -> Result<(), ServerError> {
        // Add the channel to the map of price streams
        {
            price_streams
                .write()
                .await
                .insert(pair_info.clone(), price_tx.clone());
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
                    let price = price_res.map_err(ServerError::ExchangeConnection)?;
                    // `send` only errors if there are no more receivers, meaning no more clients
                    // are subscribed to this price stream. In this case, we remove the stream from
                    // the global map, and complete the task.
                    if price_tx.send(price).is_err() {
                        price_streams.write().await.remove(&pair_info);
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Fetch a price stream for the given pair info from the global map
    pub async fn get_or_create_price_stream(
        &mut self,
        pair_info: PairInfo,
        config: &ExchangeConnectionsConfig,
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
                                handle_ws_message(msg_inner, &mut subscriptions, &mut write_stream, global_price_streams.clone(), &config).await?;
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

    Ok(())
}

/// Handles an incoming websocket message
async fn handle_ws_message(
    message: Message,
    subscriptions: &mut PriceStreamMap,
    write_stream: &mut WsWriteStream,
    global_price_streams: GlobalPriceStreams,
    config: &ExchangeConnectionsConfig,
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
    config: &ExchangeConnectionsConfig,
) -> Result<SubscriptionResponse, ServerError> {
    match message {
        WebsocketMessage::Subscribe { topic } => {
            let pair_info = parse_pair_info_from_topic(&topic)?;
            let price_stream =
                global_price_streams.get_or_create_price_stream(pair_info.clone(), config).await?;
            subscriptions.insert(pair_info, price_stream);
        },
        WebsocketMessage::Unsubscribe { topic } => {
            let pair_info = parse_pair_info_from_topic(&topic)?;
            // TODO: We should keep track of the # of listeners for a given pair info &
            // remove the associated stream from the global map if that reaches 0
            // (provided that it is not one of the `DEFAULT_PAIRS`)
            subscriptions.remove(&pair_info);
        },
    };

    Ok(SubscriptionResponse { subscriptions: get_subscribed_topics(subscriptions) })
}
