//! Definitions of errors that can occur in the price reporter server.

use std::{
    error::Error,
    fmt::{self, Display},
};

use price_reporter::errors::ExchangeConnectionError;
use serde_json::Error as SerdeError;
use tokio::task::JoinError;

/// An error that can occur in the price reporter server.
#[derive(Debug)]
pub enum ServerError {
    /// An error setting up the token remapping
    TokenRemap(String),
    /// An error attempting to subscribe to a price stream
    /// from an invalid exchange
    InvalidExchange(String),
    /// An error establishing a connection to an exchange
    ExchangeConnection(ExchangeConnectionError),
    /// An error streaming prices from an exchange
    PriceStreaming(String),
    /// An error establishing a websocket connection
    WebsocketConnection(String),
    /// An error sending a message over a websocket
    WebsocketSend(String),
    /// An error receiving a message over a websocket
    WebsocketReceive(String),
    /// An error during de/serialization
    Serde(SerdeError),
    /// An error joining a task
    JoinError(JoinError),
}

impl Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for ServerError {}
