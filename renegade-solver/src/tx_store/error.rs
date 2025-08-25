//! Defines the error types for the transaction store.

use thiserror::Error;

/// Type alias for Results using TxStoreError
pub type TxStoreResult<T> = Result<T, TxStoreError>;

/// The generic tx store error
#[derive(Error, Debug)]
pub enum TxStoreError {
    /// The transaction was not found.
    #[error("tx not found: {id}")]
    TxNotFound {
        /// The ID of the transaction in the internal tx store.
        id: String,
    },
    /// The transaction request is invalid.
    #[error("tx request invalid: {0}")]
    TxRequestInvalid(String),
}
