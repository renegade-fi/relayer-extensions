//! Definitions of errors that can occur in the price reporter server.

use std::fmt::{self, Display};

use price_reporter::errors::PriceReporterError;
use serde_json::Error as SerdeError;

/// An error that can occur in the price reporter server.
#[derive(Debug)]
pub enum ServerError {
    /// An error setting up the token remapping
    TokenRemap(String),
    /// An error fetching a price stream when the global map is uninitialized
    PriceStreamsUninitialized,
    /// An error with the price reporter
    _PriceReporter(PriceReporterError),
    /// An error establishing a websocket connection
    WebsocketConnection(String),
    /// An error sending a message over a websocket
    WebsocketSend(String),
    /// An error receiving a message over a websocket
    WebsocketReceive(String),
    /// An error during de/serialization
    Serde(SerdeError),
}

impl Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
