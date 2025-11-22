//! Message queue error definitions

/// Message queue errors
#[derive(Debug, thiserror::Error)]
pub enum MessageQueueError {
    /// An error sending a message
    #[error("error sending message: {0}")]
    Send(String),
    /// An error polling messages
    #[error("error polling messages: {0}")]
    Poll(String),
    /// An error deleting a message
    #[error("error deleting message: {0}")]
    Delete(String),
    /// An error de/serializing a value
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

#[allow(clippy::needless_pass_by_value)]
impl MessageQueueError {
    /// Create a new send error
    pub fn send<T: ToString>(msg: T) -> Self {
        Self::Send(msg.to_string())
    }

    /// Create a new poll error
    pub fn poll<T: ToString>(msg: T) -> Self {
        Self::Poll(msg.to_string())
    }

    /// Create a new delete error
    pub fn delete<T: ToString>(msg: T) -> Self {
        Self::Delete(msg.to_string())
    }
}
