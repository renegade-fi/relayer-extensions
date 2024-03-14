//! Miscellaneous utility types and helper functions.

use std::str::FromStr;

use common::types::{exchange::Exchange, token::Token, Price};
use futures_util::stream::SplitSink;
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::broadcast::{Receiver, Sender},
};
use tokio_stream::{wrappers::BroadcastStream, StreamMap};
use tokio_tungstenite::WebSocketStream;
use tungstenite::Message;
use util::err_str;

use crate::errors::ServerError;

// ---------
// | TYPES |
// ---------

/// A type alias for a tuple of (exchange, base token, quote token)
pub type PairInfo = (Exchange, Token, Token);

/// A type alias for the sender end of a price channel
pub type PriceSender = Sender<Price>;

/// A type alias for the receiver end of a price channel
pub type PriceReceiver = Receiver<Price>;

/// A type alias for a price stream
pub type PriceStream = BroadcastStream<Price>;

/// A type alias for a map of price streams, indexed by the (source, base,
/// quote) tuple
pub type PriceStreamMap = StreamMap<PairInfo, PriceStream>;

/// A type alias for a websocket write stream
pub type WsWriteStream = SplitSink<WebSocketStream<TcpStream>, Message>;

/// A message that is sent by the price reporter to the client indicating
/// a price udpate for the given topic
#[derive(Serialize, Deserialize)]
pub struct PriceMessage {
    /// The topic for which the price update is being sent
    pub topic: String,
    /// The new price
    pub price: Price,
}

// -----------
// | HELPERS |
// -----------

/// Get the topic name for a given pair info
pub fn get_pair_info_topic(pair_info: &PairInfo) -> String {
    format!("{}-{}-{}", pair_info.0, pair_info.1, pair_info.2)
}

/// Parse the pair info from a given topic
pub fn parse_pair_info_from_topic(topic: &str) -> Result<PairInfo, ServerError> {
    let parts: Vec<&str> = topic.split('-').collect();
    let exchange = Exchange::from_str(parts[0]).map_err(err_str!(ServerError::InvalidExchange))?;
    let base = Token::from_addr(parts[1]);
    let quote = Token::from_addr(parts[2]);

    Ok((exchange, base, quote))
}

/// Get all the topics that are subscribed to in a `PriceStreamMap`
pub fn get_subscribed_topics(subscriptions: &PriceStreamMap) -> Vec<String> {
    subscriptions.keys().map(get_pair_info_topic).collect()
}
