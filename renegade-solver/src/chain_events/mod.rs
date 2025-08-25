//! Defines and implements the worker that listens for on-chain events
//!
//! The event listener is responsible for:
//! - Observing tx inclusion and recording metrics related to win rate and
//!   latency
pub mod listener;
