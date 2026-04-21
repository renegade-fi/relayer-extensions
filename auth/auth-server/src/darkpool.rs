//! Local alias for the chain-specific darkpool client
//!
//! Defined here (instead of using `renegade_darkpool_client::DarkpoolClient`)
//! because workspace builds pull in funds-manager, which requires the
//! `all-chains` feature on `darkpool-client`. With `all-chains` active, the
//! upstream single-chain `DarkpoolClient` type alias is not exported.

use renegade_darkpool_client::client::DarkpoolClientInner;

/// The chain-specific darkpool client used by `auth-server`
#[cfg(all(feature = "arbitrum", not(feature = "base")))]
pub(crate) type DarkpoolClient =
    DarkpoolClientInner<renegade_darkpool_client::arbitrum::ArbitrumDarkpool>;

/// The chain-specific darkpool client used by `auth-server`
#[cfg(feature = "base")]
pub(crate) type DarkpoolClient = DarkpoolClientInner<renegade_darkpool_client::base::BaseDarkpool>;
