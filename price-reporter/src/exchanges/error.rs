use renegade_common::types::{exchange::Exchange, token::Token};
use thiserror::Error;

/// The core error type used by the ExchangeConnection. All thrown errors are
/// handled by the PriceReporter, either for restarts or panics upon too many
/// consecutive errors.
#[derive(Clone, Debug, Error)]
pub enum ExchangeConnectionError {
    /// A websocket remote connection hangup.
    #[error("remote connection hangup: {0}")]
    ConnectionHangup(String),
    /// A cryptographic error occurred
    #[error("cryptographic error: {0}")]
    Crypto(String),
    /// An initial websocket subscription to a remote server failed.
    #[error("initial websocket subscription failed: {0}")]
    HandshakeFailure(String),
    /// Could not parse a remote server message.
    #[error("could not parse remote server message: {0}")]
    InvalidMessage(String),
    /// The maximum retry count was exceeded while trying to re-establish
    /// an exchange connection
    #[error(
        "maximum retry count exceeded while trying to re-establish an exchange connection to {0}"
    )]
    MaxRetries(Exchange),
    /// The given pair is not supported by the exchange
    #[error("the given pair ({0}, {1}) is not supported by the exchange ({2})")]
    UnsupportedPair(Token, Token, Exchange),
    /// Error sending on the `write` end of the websocket
    #[error("error sending on the `write` end of the websocket: {0}")]
    SendError(String),
    /// Error saving the state of a price stream
    #[error("error saving the state of a price stream: {0}")]
    SaveState(String),
    /// Tried to initialize an ExchangeConnection that was already initialized
    #[error("tried to initialize an ExchangeConnection that was already initialized: {0}")]
    AlreadyInitialized(Exchange, Token, Token),
}
