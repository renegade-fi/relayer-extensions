//! Miscellaneous utility types and helper functions.

use std::{collections::HashMap, str::FromStr, sync::Arc};

use common::types::{exchange::Exchange, token::Token, Price};
use futures_util::stream::SplitSink;
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::{broadcast::Sender, mpsc::UnboundedSender, RwLock},
};
use tokio_stream::{wrappers::BroadcastStream, StreamMap};
use tokio_tungstenite::WebSocketStream;
use tungstenite::Message;
use util::err_str;

use crate::errors::ServerError;

// ----------
// | CONSTS |
// ----------

/// The number of milliseconds to wait in between sending keepalive messages to
/// the connections
pub const KEEPALIVE_INTERVAL_MS: u64 = 15_000; // 15 seconds
/// The number of milliseconds to wait in between retrying connections
pub const CONN_RETRY_DELAY_MS: u64 = 2_000; // 2 seconds
/// The number of milliseconds in which `MAX_CONN_RETRIES` failures will cause a
/// failure of the price reporter
pub const MAX_CONN_RETRY_WINDOW_MS: u64 = 60_000; // 1 minute
/// The maximum number of retries to attempt before giving up on a connection
pub const MAX_CONN_RETRIES: usize = 5;

// ---------
// | TYPES |
// ---------

/// A type alias for a tuple of (exchange, base token, quote token)
pub type PairInfo = (Exchange, Token, Token);

/// A type alias for the sender end of a price channel
pub type PriceSender = Sender<Price>;

/// A type alias for a shareable map of price streams, indexed by the (source,
/// base, quote) tuple
pub type SharedPriceStreams = Arc<RwLock<HashMap<PairInfo, PriceSender>>>;

/// A type alias for a price stream
pub type PriceStream = BroadcastStream<Price>;

/// A type alias for a mapped stream prices, indexed by the (source, base,
/// quote) tuple
pub type PriceStreamMap = StreamMap<PairInfo, PriceStream>;

/// A type alias for a websocket write stream
pub type WsWriteStream = SplitSink<WebSocketStream<TcpStream>, Message>;

/// A type alias for the sender end of a price stream closure channel
pub type ClosureSender = UnboundedSender<Result<(), ServerError>>;

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
