//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::{collections::HashMap, sync::Arc, time::Duration};

use external_api::websocket::{SubscriptionResponse, WebsocketMessage};
use futures_util::{SinkExt, StreamExt};
use price_reporter::{
    exchange::connect_exchange, reporter::KEEPALIVE_INTERVAL_MS, worker::ExchangeConnectionsConfig,
};
use tokio::{
    net::TcpStream,
    sync::{broadcast::channel, RwLock},
    task::JoinHandle,
    time::Instant,
};
use tokio_stream::StreamMap;
use tokio_tungstenite::accept_async;
use tungstenite::Message;
use util::err_str;

use crate::{
    errors::ServerError,
    utils::{
        get_pair_info_topic, get_subscribed_topics, parse_pair_info_from_topic, PairInfo,
        PriceMessage, PriceReceiver, PriceSender, PriceStream, PriceStreamMap, WsWriteStream,
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
    pub price_streams: Arc<RwLock<HashMap<PairInfo, PriceSender>>>,
}

impl GlobalPriceStreams {
    /// Initialize a price stream for the given pair info
    pub async fn init_price_stream(
        &self,
        pair_info: PairInfo,
        config: &ExchangeConnectionsConfig,
    ) -> Result<PriceStream, ServerError> {
        let (exchange, base, quote) = pair_info.clone();

        // Connect to the pair on the specified exchange
        let mut conn = connect_exchange(&base, &quote, config, exchange)
            .await
            .map_err(ServerError::ExchangeConnection)?;

        // Create a shared channel into which we forward streamed prices
        let (tx, rx) = channel(32 /* capacity */);

        // Add the channel to the map of price streams
        {
            self.price_streams.write().await.insert(pair_info, tx.clone());
        }

        // Spawn a task responsible for forwarding prices into the broadcast channel &
        // sending keepalive messages to the exchange
        // TODO: If/when this task fails, the price reporter should shut down
        let _handle: JoinHandle<Result<(), ServerError>> = tokio::spawn(async move {
            let delay = tokio::time::sleep(Duration::from_millis(KEEPALIVE_INTERVAL_MS));
            tokio::pin!(delay);

            loop {
                let res = tokio::select! {
                    // Send a keepalive message to the exchange
                    _ = &mut delay => {
                        conn.send_keepalive().await.map_err(ServerError::ExchangeConnection)?;
                        delay.as_mut().reset(Instant::now() + Duration::from_millis(KEEPALIVE_INTERVAL_MS));
                        Result::<(), ServerError>::Ok(())
                    }

                    // Forward the next price into the broadcast channel
                    Some(price_res) = conn.next() => {
                        let price = price_res.map_err(ServerError::ExchangeConnection)?;
                        let _num_listeners = tx.send(price).map_err(err_str!(ServerError::PriceStreaming))?;
                        Result::<(), ServerError>::Ok(())
                    }
                };

                if res.is_err() {
                    break res;
                }
            }
        });

        // Return a handle to the broadcast channel stream
        Ok(PriceStream::new(rx))
    }

    /// Fetch a price stream for the given pair info from the global map
    pub async fn get_or_create_price_stream(
        &self,
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

/// The price reporter server
pub struct Server {
    /// The configuration for the exchange connections
    pub config: ExchangeConnectionsConfig,
    /// The global map of price streams, shared across all connections
    pub global_price_streams: GlobalPriceStreams,
    /// The receivers of the channels for the default price streams,
    /// kept so that the channels are not dropped
    pub default_receivers: Vec<PriceReceiver>,
}

impl Server {
    /// Initialize the price reporter server
    pub async fn new(config: ExchangeConnectionsConfig) -> Result<Self, ServerError> {
        let global_price_streams =
            GlobalPriceStreams { price_streams: Arc::new(RwLock::new(HashMap::new())) };
        let default_receivers = Vec::new();

        // TODO: Connect to `DEFAULT_PAIRS` and store the receivers in
        // `default_receivers``

        Ok(Server { config, global_price_streams, default_receivers })
    }
}

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
    global_price_streams: GlobalPriceStreams,
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
