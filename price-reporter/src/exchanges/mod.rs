//! Exchange connection shims

mod binance;
mod coinbase;
mod connection;
mod error;
mod kraken;
mod okx;
mod util;

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
