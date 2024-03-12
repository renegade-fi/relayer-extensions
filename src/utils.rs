//! Miscellaneous utility types and helper functions.

use std::str::FromStr;

use common::types::{exchange::Exchange, token::Token};
use futures_util::stream::SplitSink;
use price_reporter::exchange::PriceStreamType;
use tokio::{net::TcpStream, sync::watch::Sender};
use tokio_stream::{wrappers::WatchStream, StreamMap};
use tokio_tungstenite::WebSocketStream;
use tungstenite::Message;

use crate::{errors::ServerError, server::PRICE_STREAMS};

// ---------
// | TYPES |
// ---------

/// A type alias for a tuple of (exchange, base token, quote token)
pub type PairInfo = (Exchange, Token, Token);

/// A type alias for the sender end of a price stream
pub type PriceSender = Sender<PriceStreamType>;

/// A type alias for a price stream
pub type PriceStream = WatchStream<PriceStreamType>;

/// A type alias for a map of price streams, indexed by the (source, base,
/// quote) tuple
pub type PriceStreamMap = StreamMap<PairInfo, PriceStream>;

/// A type alias for a websocket write stream
pub type WsWriteStream = SplitSink<WebSocketStream<TcpStream>, Message>;

// -----------
// | HELPERS |
// -----------

/// Get the topic name for a given pair info
pub fn get_pair_info_topic(pair_info: &PairInfo) -> String {
    format!("{}-{}-{}", pair_info.0, pair_info.1, pair_info.2)
}

/// Parse the pair info from a given topic
pub fn parse_pair_info_from_topic(topic: &str) -> PairInfo {
    let parts: Vec<&str> = topic.split('-').collect();
    let exchange = Exchange::from_str(parts[0]).unwrap();
    let base = Token::from_addr(parts[1]);
    let quote = Token::from_addr(parts[2]);

    (exchange, base, quote)
}

/// Fetch a price stream for the given pair info from the global map
pub fn get_price_stream(pair_info: &PairInfo) -> Result<PriceStream, ServerError> {
    let price_streams = PRICE_STREAMS.get().ok_or(ServerError::PriceStreamsUninitialized)?;

    let price_stream = if let Some(stream_tx) = price_streams.get(pair_info) {
        PriceStream::new(stream_tx.subscribe())
    } else {
        // TODO: If the price stream doesn't exist, attempt to create it
        todo!()
    };

    Ok(price_stream)
}

/// Get all the topics that are subscribed to in a `PriceStreamMap`
pub fn get_subscribed_topics(subscriptions: &PriceStreamMap) -> Vec<String> {
    subscriptions.keys().map(get_pair_info_topic).collect()
}
