//! Definitions of errors that can occur in the price reporter server.

use std::io;

use crate::exchanges::error::ExchangeConnectionError;

/// An error that can occur in the price reporter server.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ServerError {
    /// An error setting up the token remapping
    #[error("Error setting up the token remapping: {0}")]
    TokenRemap(String),
    /// An error attempting to subscribe to a price stream
    /// for an invalid (exchange, base, quote) tuple
    #[error("Invalid (exchange, base, quote) tuple: {0}")]
    InvalidPairInfo(String),
    /// An error establishing a connection to an exchange
    #[error("Error establishing connection to {0}")]
    ExchangeConnection(ExchangeConnectionError),
    /// An error getting the peer address of a websocket connection
    #[error("Error getting peer address: {0}")]
    GetPeerAddr(io::Error),
    /// An error establishing a websocket connection
    #[error("Error establishing websocket connection: {0}")]
    WebsocketConnection(String),
    /// An error sending a message over a websocket
    #[error("Error sending message over websocket: {0}")]
    WebsocketSend(String),
    /// An error receiving a message over a websocket
    #[error("Error receiving message over websocket: {0}")]
    WebsocketReceive(String),
    /// An error during de/serialization
    #[error("Error during de/serialization: {0}")]
    Serde(String),
    /// An error in the HTTP server execution
    #[error("Error in HTTP server: {0}")]
    HttpServer(String),
    /// An error in the authorization of an HTTP request
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    /// An error indicating that the admin key was not provided
    #[error("No admin key provided")]
    NoAdminKey,
}
