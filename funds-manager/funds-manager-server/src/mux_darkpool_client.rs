//! A wrapper around the `DarkpoolClientInner` struct that muxes the
//! implementation logic between an Arbitrum darkpool client and a Base darkpool
//! client

use std::fmt::{self, Display};

use alloy::contract::Event;
use alloy_primitives::ChainId;
use alloy_sol_types::SolEvent;
use renegade_circuit_types::wallet::Nullifier;
use renegade_common::types::chain::Chain;
use renegade_darkpool_client::{
    arbitrum::ArbitrumDarkpool,
    base::BaseDarkpool,
    client::{DarkpoolClientConfig, DarkpoolClientInner, RenegadeProvider},
    errors::DarkpoolClientError,
};

/// The error type returned by the MuxDarkpoolClient
pub enum MuxDarkpoolClientError {
    /// An error that occurred while interacting with the darkpool client
    Client(DarkpoolClientError),
    /// An error attempting to use an unsupported chain
    UnsupportedChain,
}

impl From<DarkpoolClientError> for MuxDarkpoolClientError {
    fn from(e: DarkpoolClientError) -> Self {
        Self::Client(e)
    }
}

impl Display for MuxDarkpoolClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(e) => write!(f, "{}", e),
            Self::UnsupportedChain => write!(f, "Unsupported chain"),
        }
    }
}

/// A wrapper providing a unified interface to the Arbitrum and Base darkpool
/// clients
#[derive(Clone)]
pub enum MuxDarkpoolClient {
    /// An Arbitrum darkpool client
    Arbitrum(DarkpoolClientInner<ArbitrumDarkpool>),
    /// A Base darkpool client
    Base(DarkpoolClientInner<BaseDarkpool>),
}

impl MuxDarkpoolClient {
    /// Create a new darkpool client
    pub fn new(chain: Chain, config: DarkpoolClientConfig) -> Result<Self, MuxDarkpoolClientError> {
        match chain {
            Chain::ArbitrumSepolia | Chain::ArbitrumOne => {
                let client = DarkpoolClientInner::<ArbitrumDarkpool>::new(config)?;
                Ok(Self::Arbitrum(client))
            },
            Chain::BaseSepolia | Chain::BaseMainnet => {
                let client = DarkpoolClientInner::<BaseDarkpool>::new(config)?;
                Ok(Self::Base(client))
            },
            _ => Err(MuxDarkpoolClientError::UnsupportedChain),
        }
    }

    /// Get a reference to some underlying RPC client
    pub fn provider(&self) -> &RenegadeProvider {
        match self {
            Self::Arbitrum(client) => client.provider(),
            Self::Base(client) => client.provider(),
        }
    }

    /// Get the chain ID
    pub async fn chain_id(&self) -> Result<ChainId, MuxDarkpoolClientError> {
        match self {
            Self::Arbitrum(client) => client.chain_id().await,
            Self::Base(client) => client.chain_id().await,
        }
        .map_err(Into::into)
    }

    /// Create an event filter
    pub fn event_filter<E: SolEvent>(&self) -> Event<&RenegadeProvider, E> {
        match self {
            Self::Arbitrum(client) => client.event_filter::<E>(),
            Self::Base(client) => client.event_filter::<E>(),
        }
    }

    // ------------------------
    // | Contract Interaction |
    // ------------------------

    /// Check whether the given nullifier is used
    ///
    /// Returns `true` if the nullifier is used, `false` otherwise
    pub async fn check_nullifier_used(
        &self,
        nullifier: Nullifier,
    ) -> Result<bool, MuxDarkpoolClientError> {
        match self {
            Self::Arbitrum(client) => client.check_nullifier_used(nullifier).await,
            Self::Base(client) => client.check_nullifier_used(nullifier).await,
        }
        .map_err(Into::into)
    }
}
