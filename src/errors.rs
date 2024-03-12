//! Definitions of errors that can occur in the price reporter server.

use std::fmt::{self, Display};

use price_reporter::errors::PriceReporterError;

/// An error that can occur in the price reporter server.
#[derive(Debug)]
pub enum ServerError {
    /// An error setting up the token remapping
    TokenRemap(String),
    /// An error with the price reporter
    _PriceReporter(PriceReporterError),
    /// An error establishing a websocket connection
    WebsocketConnection(String),
    /// An error sending a message over a websocket
    WebsocketSend(String),
}

impl Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
