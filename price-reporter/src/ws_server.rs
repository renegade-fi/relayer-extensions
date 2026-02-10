//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use renegade_api::websocket::{SubscriptionResponse, WebsocketMessage};
use renegade_util::err_str;
use tokio::net::TcpStream;
use tokio_stream::StreamMap;
use tokio_tungstenite::accept_async;
use tracing::{debug, info};
use tungstenite::Message;

use crate::{
    errors::ServerError,
    exchanges::ExchangeConnectionsConfig,
    price_stream_manager::GlobalPriceStreams,
    utils::{
        PairInfo, PriceMessage, PriceStreamMap, WsWriteStream, get_price_topic_str,
        get_subscribed_topics,
    },
};

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
