//! Defines and implements the worker that listens for on-chain events
//!
//! The event listener is responsible for:
//! - Metrics: listening for nullifier spent events and recording metrics
//!   related to settlement volume
mod error;
pub mod listener;
mod tasks;
pub mod worker;
