//! Definitions of errors that can occur in the price reporter server.

use std::{
    error::Error,
    fmt::{self, Display},
    io,
};

use renegade_price_reporter::errors::ExchangeConnectionError;

/// An error that can occur in the price reporter server.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ServerError {
    /// An error setting up the token remapping
    TokenRemap(String),
    /// An error attempting to subscribe to a price stream
    /// for an invalid (exchange, base, quote) tuple
    InvalidPairInfo(String),
    /// An error establishing a connection to an exchange
    ExchangeConnection(ExchangeConnectionError),
    /// An error getting the peer address of a websocket connection
    GetPeerAddr(io::Error),
    /// An error establishing a websocket connection
    WebsocketConnection(String),
    /// An error sending a message over a websocket
    WebsocketSend(String),
    /// An error receiving a message over a websocket
    WebsocketReceive(String),
    /// An error during de/serialization
    Serde(String),
    /// An error in the HTTP server execution
    HttpServer(String),
    /// An error in the authorization of an HTTP request
    Unauthorized(String),
    /// An error indicating that the admin key was not provided
    NoAdminKey,
}

impl Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl Error for ServerError {}
