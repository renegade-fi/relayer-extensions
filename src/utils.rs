//! Miscellaneous utility types and helper functions.

use std::{collections::HashMap, env, str::FromStr, sync::Arc};

use arbitrum_client::constants::Chain;
use common::types::{exchange::Exchange, token::Token, Price};
use futures_util::stream::SplitSink;
use matchit::Router;
use price_reporter::worker::ExchangeConnectionsConfig;
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::{broadcast::Sender, mpsc::UnboundedSender, RwLock},
};
use tokio_stream::{wrappers::BroadcastStream, StreamMap};
use tokio_tungstenite::WebSocketStream;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};
use tungstenite::Message;
use util::err_str;

use crate::{errors::ServerError, http_server::routes::Handler};

// ----------
// | CONSTS |
// ----------

/// The number of milliseconds to wait in between sending keepalive messages to
/// the connections
pub const KEEPALIVE_INTERVAL_MS: u64 = 15_000; // 15 seconds
/// The number of milliseconds to wait in between retrying connections
pub const CONN_RETRY_DELAY_MS: u64 = 2_000; // 2 seconds
/// The number of milliseconds in which `MAX_CONN_RETRIES` failures will cause a
/// failure of the price reporter
pub const MAX_CONN_RETRY_WINDOW_MS: u64 = 60_000; // 1 minute
/// The maximum number of retries to attempt before giving up on a connection
pub const MAX_CONN_RETRIES: usize = 5;

/// The name of the environment variable specifying the port on which the
/// server listens for incoming websocket connections
const WS_PORT_ENV_VAR: &str = "WS_PORT";
/// The default port on which the server listens for incoming websocket
/// connections
const DEFAULT_WS_PORT: u16 = 4000;
/// The name of the environment variable specifying the port on which the
/// server listens for http requests
const HTTP_PORT_ENV_VAR: &str = "HTTP_PORT";
/// The default port on which the server listens for http requests
const DEFAULT_HTTP_PORT: u16 = 3000;
/// The name of the environment variable specifying the path to the token
/// remap file
const TOKEN_REMAP_PATH_ENV_VAR: &str = "TOKEN_REMAP_PATH";
/// The name of the environment variable specifying the chain to use
/// for token remapping
const CHAIN_ID_ENV_VAR: &str = "CHAIN_ID";
/// The default chain to use for token remapping
const DEFAULT_CHAIN: Chain = Chain::Testnet;
/// The name of the environment variable specifying the Coinbase
/// API key
const CB_API_KEY_ENV_VAR: &str = "CB_API_KEY";
/// The name of the environment variable specifying the Coinbase
/// API secret
const CB_API_SECRET_ENV_VAR: &str = "CB_API_SECRET";
/// The name of the environment variable specifying the Ethereum
/// RPC node websocket address
const ETH_WS_ADDR_ENV_VAR: &str = "ETH_WS_ADDR";

// ---------
// | TYPES |
// ---------

/// A type alias for a tuple of (exchange, base token, quote token)
pub type PairInfo = (Exchange, Token, Token);

/// A type alias for the sender end of a price channel
pub type PriceSender = Sender<Price>;

/// A type alias for a shareable map of price streams, indexed by the (source,
/// base, quote) tuple
pub type SharedPriceStreams = Arc<RwLock<HashMap<PairInfo, PriceSender>>>;

/// A type alias for a price stream
pub type PriceStream = BroadcastStream<Price>;

/// A type alias for a mapped stream prices, indexed by the (source, base,
/// quote) tuple
pub type PriceStreamMap = StreamMap<PairInfo, PriceStream>;

/// A type alias for a websocket write stream
pub type WsWriteStream = SplitSink<WebSocketStream<TcpStream>, Message>;

/// A type alias for the sender end of a price stream closure channel
pub type ClosureSender = UnboundedSender<Result<(), ServerError>>;

/// A type alias for URL parameters
pub type UrlParams = HashMap<String, String>;

/// A type alias for a router which matches URLs to handlers
pub type HttpRouter = Router<Box<dyn Handler>>;

/// A message that is sent by the price reporter to the client indicating
/// a price udpate for the given topic
#[derive(Serialize, Deserialize)]
pub struct PriceMessage {
    /// The topic for which the price update is being sent
    pub topic: String,
    /// The new price
    pub price: Price,
}

/// The configuration options for the price reporter server
pub struct PriceReporterConfig {
    /// The port on which the server listens for incoming websocket connections
    pub ws_port: u16,
    /// The port on which the server listens for incoming http requests
    pub http_port: u16,
    /// The path to the token remap file
    pub token_remap_path: Option<String>,
    /// The chain to use for token remapping
    pub remap_chain: Chain,
    /// The configuration options that may be used by exchange connections
    pub exchange_conn_config: ExchangeConnectionsConfig,
}

// -----------
// | HELPERS |
// -----------

/// Configure the logging subscriber
pub fn setup_logging() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy(),
        )
        .with(fmt::layer().with_file(true).with_line_number(true).json().flatten_event(true))
        .init();
}

/// Parse the configuration options from environment variables
pub fn parse_config_env_vars() -> PriceReporterConfig {
    let ws_port = env::var(WS_PORT_ENV_VAR).map(|p| p.parse().unwrap()).unwrap_or(DEFAULT_WS_PORT);
    let http_port =
        env::var(HTTP_PORT_ENV_VAR).map(|p| p.parse().unwrap()).unwrap_or(DEFAULT_HTTP_PORT);
    let token_remap_path = env::var(TOKEN_REMAP_PATH_ENV_VAR).ok();
    let remap_chain =
        env::var(CHAIN_ID_ENV_VAR).map(|c| c.parse().unwrap()).unwrap_or(DEFAULT_CHAIN);
    let coinbase_api_key = env::var(CB_API_KEY_ENV_VAR).ok();
    let coinbase_api_secret = env::var(CB_API_SECRET_ENV_VAR).ok();
    let eth_websocket_addr = env::var(ETH_WS_ADDR_ENV_VAR).ok();

    PriceReporterConfig {
        ws_port,
        http_port,
        token_remap_path,
        remap_chain,
        exchange_conn_config: ExchangeConnectionsConfig {
            coinbase_api_key,
            coinbase_api_secret,
            eth_websocket_addr,
        },
    }
}

/// Get the topic name for a given pair info
pub fn get_pair_info_topic(pair_info: &PairInfo) -> String {
    format!("{}-{}-{}", pair_info.0, pair_info.1, pair_info.2)
}

/// Parse the pair info from a given topic
pub fn parse_pair_info_from_topic(topic: &str) -> Result<PairInfo, ServerError> {
    let parts: Vec<&str> = topic.split('-').collect();
    let exchange = Exchange::from_str(parts[0]).map_err(err_str!(ServerError::InvalidPairInfo))?;
    let base = Token::from_addr(parts[1]);
    let quote = Token::from_addr(parts[2]);

    Ok((exchange, base, quote))
}

/// Get all the topics that are subscribed to in a `PriceStreamMap`
pub fn get_subscribed_topics(subscriptions: &PriceStreamMap) -> Vec<String> {
    subscriptions.keys().map(get_pair_info_topic).collect()
}

/// Validate a pair info tuple, checking that the exchange supports the base
/// and quote tokens
pub fn validate_subscription(topic: &str) -> Result<PairInfo, ServerError> {
    let (exchange, base, quote) = parse_pair_info_from_topic(topic)?;

    if base == quote {
        return Err(ServerError::InvalidPairInfo(
            "Base and quote tokens must be different".to_string(),
        ));
    }

    if exchange == Exchange::UniswapV3 {
        return Err(ServerError::InvalidPairInfo("UniswapV3 is not supported".to_string()));
    }

    let base_exchanges = base.supported_exchanges();
    let quote_exchanges = quote.supported_exchanges();

    if !(base_exchanges.contains(&exchange) && quote_exchanges.contains(&exchange)) {
        return Err(ServerError::InvalidPairInfo(format!(
            "{} does not support the pair ({}, {})",
            exchange, base, quote
        )));
    }

    // TODO: Subscription auth - API key?

    Ok((exchange, base, quote))
}
