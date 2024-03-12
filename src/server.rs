//! The core websocket server of the price reporter, handling subscriptions to
//! price streams

use std::{collections::HashMap, sync::OnceLock};

use common::types::{exchange::Exchange, token::Token};
use futures_util::{SinkExt, StreamExt};
use tokio::{net::TcpStream, sync::watch::channel};
use tokio_stream::{wrappers::WatchStream, StreamMap};
use tokio_tungstenite::accept_async;
use tungstenite::Message;
use util::err_str;

use crate::{
    errors::ServerError,
    utils::{PairInfo, PriceSender},
};

// ----------------
// | GLOBAL STATE |
// ----------------

/// A map of price streams, indexed by the (source, base, quote) tuple
static PRICE_STREAMS: OnceLock<HashMap<PairInfo, PriceSender>> = OnceLock::new();

/// Initializes the global price streams map
pub fn init_price_streams() {
    PRICE_STREAMS.get_or_init(|| {
        // TODO: Initialize w/ DEFAULT_PAIRS from relayer

        let dummy_price_tuple =
            (Exchange::Binance, Token::from_ticker("WETH"), Token::from_ticker("USDC"));

        let (dummy_tx, _) = channel(Ok(1.0));

        let mut initial_price_streams = HashMap::new();
        initial_price_streams.insert(dummy_price_tuple, dummy_tx);

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
    let (mut write_stream, _) = websocket_stream.split();

    let mut subscriptions = StreamMap::new();

    let dummy_price_tuple =
        (Exchange::Binance, Token::from_ticker("WETH"), Token::from_ticker("USDC"));
    let dummy_price_stream =
        WatchStream::new(PRICE_STREAMS.get().unwrap().get(&dummy_price_tuple).unwrap().subscribe());
    subscriptions.insert(dummy_price_tuple.clone(), dummy_price_stream);

    loop {
        tokio::select! {
            Some((_price_tuple, price)) = subscriptions.next() => {
                write_stream.send(Message::Text(format!("{:?}", price))).await.map_err(err_str!(ServerError::WebsocketSend))?;
            }

            // TODO: Listen for subscription requests
        }
    }
}
