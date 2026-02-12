//! A client for interacting with the darkpool.
//!
//! This is a TEMPORARY module which will later be ported over to the relayer
//! repository.

use alloy::{primitives::Address, providers::DynProvider};
use renegade_solidity_abi::v2::IDarkpoolV2::IDarkpoolV2Instance;

pub mod error;
pub mod indexing;
pub mod utils;

/// A client for interacting with the darkpool
#[derive(Clone)]
pub struct DarkpoolClient {
    /// The darkpool contract instance
    pub darkpool: IDarkpoolV2Instance<DynProvider>,
}

impl DarkpoolClient {
    /// Create a new darkpool client
    pub fn new(darkpool: IDarkpoolV2Instance<DynProvider>) -> Self {
        Self { darkpool }
    }

    /// Get a reference to the underlying RPC provider
    pub fn provider(&self) -> &DynProvider {
        self.darkpool.provider()
    }

    /// Get the address of the darkpool contract
    pub fn darkpool_address(&self) -> Address {
        *self.darkpool.address()
    }
}
