//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::{collections::HashMap, sync::OnceLock};

use common::types::{exchange::Exchange, token::Token};
use external_api::websocket::{SubscriptionResponse, WebsocketMessage};
use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpStream, sync::watch::channel};
use tokio_stream::StreamMap;
use tokio_tungstenite::accept_async;
use tungstenite::Message;
use util::err_str;

use crate::{
    errors::ServerError,
    utils::{
        get_price_stream, get_subscribed_topics, parse_pair_info_from_topic, PairInfo, PriceSender,
        PriceStreamMap, WsWriteStream,
    },
};

// ----------------
// | GLOBAL STATE |
// ----------------

/// A map of price streams, indexed by the (source, base, quote) tuple
pub static PRICE_STREAMS: OnceLock<HashMap<PairInfo, PriceSender>> = OnceLock::new();

/// Initializes the global price streams map
pub fn init_price_streams() {
    PRICE_STREAMS.get_or_init(|| {
        // TODO: Initialize w/ DEFAULT_PAIRS from relayer

        let dummy_pair_info =
            (Exchange::Binance, Token::from_ticker("WETH"), Token::from_ticker("USDC"));

        let (dummy_tx, _) = channel(Ok(1.0));

        let mut initial_price_streams = HashMap::new();
        initial_price_streams.insert(dummy_pair_info, dummy_tx);

        initial_price_streams
    });
}

// ----------
// | SERVER |
// ----------

/// Handles an incoming websocket connection,
/// establishing a listener loop for subscription requests
pub async fn handle_connection(stream: TcpStream) -> Result<(), ServerError> {
    let websocket_stream =
        accept_async(stream).await.map_err(err_str!(ServerError::WebsocketConnection))?;
    let (mut write_stream, mut read_stream) = websocket_stream.split();

    let mut subscriptions = StreamMap::new();

    loop {
        tokio::select! {
            // Send the next price to the client
            Some((_pair_info, price)) = subscriptions.next() => {
                write_stream.send(Message::Text(format!("{:?}", price))).await.map_err(err_str!(ServerError::WebsocketSend))?;
            }

            // Handle incoming websocket messages
            message = read_stream.next() => {
                match message {
                    Some(msg) => {
                        let msg_inner = msg.map_err(err_str!(ServerError::WebsocketReceive))?;

                        match msg_inner {
                            Message::Close(_) => break,
                            _ => {
                                handle_ws_message(msg_inner, &mut subscriptions, &mut write_stream).await?;
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
) -> Result<(), ServerError> {
    if let Message::Text(msg_text) = message {
        let msg_deser: Result<WebsocketMessage, _> = serde_json::from_str(&msg_text);
        let resp = match msg_deser {
            // Valid message body
            Ok(msg) => {
                let response = match handle_subscription_message(msg, subscriptions).await {
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
) -> Result<SubscriptionResponse, ServerError> {
    match message {
        WebsocketMessage::Subscribe { topic } => {
            let pair_info = parse_pair_info_from_topic(&topic);
            let price_stream = get_price_stream(&pair_info)?;
            subscriptions.insert(pair_info, price_stream);
        },
        WebsocketMessage::Unsubscribe { topic } => {
            let pair_info = parse_pair_info_from_topic(&topic);
            // TODO: We should keep track of the # of listeners for a given pair info &
            // remove the associated stream from the global map if that reaches 0
            // (provided that it is not one of the `DEFAULT_PAIRS`)
            subscriptions.remove(&pair_info);
        },
    };

    Ok(SubscriptionResponse { subscriptions: get_subscribed_topics(subscriptions) })
}
