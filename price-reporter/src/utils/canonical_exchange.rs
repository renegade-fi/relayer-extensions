//! Utilities for canonical exchange mapping
//!
//! The canonical exchange is the exchange that is used to fetch prices for a
//! given token.
//!
//! We assume that each token has a single canonical exchange, which is
//! chain-agnostic.
use std::{
    collections::HashMap,
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use renegade_common::types::{chain::Chain, exchange::Exchange};
use renegade_config::{fetch_remap_from_repo, parse_remap_from_file};
use renegade_util::{concurrency::RwStatic, err_str};

use crate::{errors::ServerError, utils::get_token_and_chain};

// ---------
// | Types |
// ---------

/// A type alias representing the mapping from a token ticker to the canonical
/// exchange to use as a price source
type CanonicalExchangeMap = HashMap<String, Exchange>;

/// The mapping from ERC-20 ticker to the canonical exchange to use as a price
/// source
static CANONICAL_EXCHANGE_MAP: RwStatic<CanonicalExchangeMap> =
    RwStatic::new(|| RwLock::new(HashMap::new()));

// -----------
// | Helpers |
// -----------

/// Get the canonical exchange for a given token ticker
pub fn get_canonical_exchange(mint: &str) -> Result<Exchange, ServerError> {
    let (token, _) = get_token_and_chain(mint)
        .ok_or_else(|| ServerError::InvalidPairInfo(format!("invalid token `{}`", mint)))?;
    let ticker = token.get_ticker().ok_or_else(|| {
        ServerError::InvalidPairInfo(format!("unable to get ticker for {}", mint))
    })?;
    let map = read_canonical_exchange_map();
    let canonical_exchange = map.get(ticker.as_str()).cloned().ok_or_else(|| {
        ServerError::InvalidPairInfo(format!("unable to get canonical exchange for {}", mint,))
    })?;

    Ok(canonical_exchange)
}

/// Get all canonical exchanges
pub fn get_canonical_exchanges() -> Vec<Exchange> {
    let map = read_canonical_exchange_map();
    map.values().copied().collect()
}

/// Set the static mapping of token tickers to the canonical exchange to use as
/// a price source
pub fn set_canonical_exchange_map(
    remap_file: Option<String>,
    chain: Chain,
) -> Result<(), ServerError> {
    let map = if let Some(file) = remap_file {
        parse_remap_from_file(file)
    } else {
        fetch_remap_from_repo(chain)
    }
    .map_err(err_str!(ServerError::TokenRemap))?;

    let chain_canonical_exchange_map = map.get_canonical_exchange_map();

    let mut global_canonical_exchange_map = write_canonical_exchange_map();

    // We extend to effectively merge the two maps. This is safe because we
    // assume each ticker has one canonical exchange.
    global_canonical_exchange_map.extend(chain_canonical_exchange_map);

    Ok(())
}

/// Returns a read lock guard to the canonical exchange map
fn read_canonical_exchange_map<'a>() -> RwLockReadGuard<'a, CanonicalExchangeMap> {
    CANONICAL_EXCHANGE_MAP.read().expect("Canonical exchange map lock poisoned")
}

/// Returns a write lock guard to the canonical exchange map
fn write_canonical_exchange_map<'a>() -> RwLockWriteGuard<'a, CanonicalExchangeMap> {
    CANONICAL_EXCHANGE_MAP.write().expect("Canonical exchange map lock poisoned")
}
