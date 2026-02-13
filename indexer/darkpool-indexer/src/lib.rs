//! The darkpool indexer's library definitions

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::unused_async)]
#![feature(let_chains)]
#![feature(trait_alias)]

pub mod api;
pub mod chain_event_listener;
pub mod cli;
pub mod crypto_mocks;
pub mod darkpool_client;
pub mod db;
pub mod indexer;
pub mod message_queue;
pub mod state_transitions;
pub mod types;
