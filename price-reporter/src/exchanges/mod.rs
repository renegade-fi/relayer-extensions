//! Exchange connection shims

use renegade_common::types::{exchange::Exchange, token::Token};

use crate::exchanges::{
    binance::BinanceConnection, coinbase::CoinbaseConnection, connection::ExchangeConnection,
    error::ExchangeConnectionError, kraken::KrakenConnection, okx::OkxConnection,
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
    base_token: &Token,
    quote_token: &Token,
    config: &ExchangeConnectionsConfig,
    exchange: Exchange,
) -> Result<Box<dyn ExchangeConnection>, ExchangeConnectionError> {
    let base_token = base_token.clone();
    let quote_token = quote_token.clone();

    Ok(match exchange {
        Exchange::Binance => {
            Box::new(BinanceConnection::connect(base_token, quote_token, config).await?)
        },
        Exchange::Coinbase => {
            Box::new(CoinbaseConnection::connect(base_token, quote_token, config).await?)
        },
        Exchange::Kraken => {
            Box::new(KrakenConnection::connect(base_token, quote_token, config).await?)
        },
        Exchange::Okx => Box::new(OkxConnection::connect(base_token, quote_token, config).await?),
        _ => return Err(ExchangeConnectionError::unsupported_exchange(exchange)),
    })
}
