//! Defines a listener for flashblocks events.

pub mod clock;
mod listener;
pub mod multi_listener;
pub use listener::{Flashblock, FlashblocksReceiver};
pub use multi_listener::FlashblocksListener;
pub mod types;
