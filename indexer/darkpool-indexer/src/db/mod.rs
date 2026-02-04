//! Database schema & interface definitions

/// The singleton row ID used for single-row tables
pub const SINGLETON_ROW_ID: i32 = 1;

#[allow(missing_docs)]
#[allow(clippy::missing_docs_in_private_items)]
pub mod schema;

pub mod client;
pub mod error;
pub mod interface;
pub mod models;
mod utils;

#[cfg(any(test, feature = "integration"))]
pub mod test_utils;
