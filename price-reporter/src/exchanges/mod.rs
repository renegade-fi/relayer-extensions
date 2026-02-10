//! Exchange connection shims

use renegade_types_core::Exchange;

use crate::{
    exchanges::{
        binance::BinanceConnection, coinbase::CoinbaseConnection, connection::ExchangeConnection,
        error::ExchangeConnectionError, kraken::KrakenConnection, okx::OkxConnection,
    },
    utils::PairInfo,
};

pub(crate) mod binance;
pub(crate) mod coinbase;
pub(crate) mod connection;
pub(crate) mod error;
pub(crate) mod kraken;
pub(crate) mod okx;
pub(crate) mod util;

/// The configuration options that may be used by exchange connections
#[derive(Clone, Debug, Default)]
pub struct ExchangeConnectionsConfig {
    /// The coinbase API key that the price reporter may use
    pub coinbase_key_name: Option<String>,
    /// The coinbase API secret that the price reporter may use
    pub coinbase_key_secret: Option<String>,
    /// The ethereum RPC node websocket addresses for on-chain data
    pub eth_websocket_addr: Option<String>,
}

impl ExchangeConnectionsConfig {
    /// Whether or not the Coinbase connection is configured
    pub fn coinbase_configured(&self) -> bool {
        self.coinbase_key_name.is_some() && self.coinbase_key_secret.is_some()
    }

    /// Whether or not the Uniswap V3 connection is configured
    pub fn uniswap_v3_configured(&self) -> bool {
        self.eth_websocket_addr.is_some()
    }
}

/// Construct a new websocket connection for the given exchange
pub async fn connect_exchange(
    pair_info: PairInfo,
    config: &ExchangeConnectionsConfig,
) -> Result<Box<dyn ExchangeConnection>, ExchangeConnectionError> {
    let exchange = pair_info.exchange;
    Ok(match exchange {
        Exchange::Binance => Box::new(BinanceConnection::connect(pair_info, config).await?),
        Exchange::Coinbase => Box::new(CoinbaseConnection::connect(pair_info, config).await?),
        Exchange::Kraken => Box::new(KrakenConnection::connect(pair_info, config).await?),
        Exchange::Okx => Box::new(OkxConnection::connect(pair_info, config).await?),
        _ => return Err(ExchangeConnectionError::unsupported_exchange(exchange)),
    })
}
