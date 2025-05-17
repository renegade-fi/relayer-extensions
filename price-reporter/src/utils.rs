//! Miscellaneous utility types and helper functions.

use std::pin::Pin;
use std::task::{Context, Poll};
use std::{collections::HashMap, env, str::FromStr, sync::Arc};

use futures_util::StreamExt;
use futures_util::{stream::SplitSink, Stream};
use itertools::Itertools;
use matchit::Router;
use renegade_common::types::{
    chain::Chain,
    exchange::Exchange,
    hmac::HmacKey,
    token::{read_token_remaps, Token},
    Price,
};
use renegade_config::setup_token_remaps;
use renegade_price_reporter::worker::ExchangeConnectionsConfig;
use renegade_util::err_str;
use serde::{Deserialize, Serialize};
use tokio::{
    net::TcpStream,
    sync::watch::{Receiver as WatchReceiver, Sender as WatchSender},
    sync::{mpsc::UnboundedSender, RwLock},
};
use tokio_stream::{wrappers::WatchStream, StreamMap};
use tokio_tungstenite::WebSocketStream;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};
use tungstenite::Message;

use crate::pair_info::PairInfo;
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
/// The name of the environment variable specifying the path to the token remap
const TOKEN_REMAP_PATH_ENV_VAR: &str = "TOKEN_REMAP_PATH";
/// The name of the environment variable specifying the chain to use
/// for token remapping
const CHAIN_ID_ENV_VAR: &str = "CHAIN_ID";
/// The default chain to use for token remapping
const DEFAULT_CHAIN: Chain = Chain::Devnet;
/// The name of the environment variable specifying the Coinbase
/// API key
const CB_API_KEY_ENV_VAR: &str = "CB_API_KEY";
/// The name of the environment variable specifying the Coinbase
/// API secret
const CB_API_SECRET_ENV_VAR: &str = "CB_API_SECRET";
/// The name of the environment variable specifying the Ethereum
/// RPC node websocket address
const ETH_WS_ADDR_ENV_VAR: &str = "ETH_WS_ADDR";
/// The name of the environment variable specifying the HMAC key for the admin
/// API
const ADMIN_KEY_ENV_VAR: &str = "ADMIN_KEY";
/// The name of the environment variable specifying the disabled exchanges
const DISABLED_EXCHANGES_ENV_VAR: &str = "DISABLED_EXCHANGES";

// ---------
// | TYPES |
// ---------

/// A type alias for a tuple of (exchange, base token, quote token)
pub type PriceTopic = (Exchange, Token, Token);

/// A type alias for the sender end of a price channel
pub type PriceSender = WatchSender<Price>;

/// A type alias for a price receiver
pub type PriceReceiver = WatchReceiver<Price>;

/// A type alias for a shareable map of price streams, indexed by the (source,
/// base, quote) tuple
pub type SharedPriceStreams = Arc<RwLock<HashMap<PairInfo, PriceReceiver>>>;

/// A type alias for a price stream
pub type SinglePriceStream = WatchStream<Price>;
/// A price stream, containing the watch underlying the stream and an optional
/// second watch for converting quote tokens
pub struct PriceStream {
    /// The watch underlying the stream
    pub stream: SinglePriceStream,
    /// The watch for converting quote tokens, if required
    pub conversion_stream: Option<SinglePriceStream>,
}

impl Stream for PriceStream {
    type Item = Price;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Poll the main stream
        let main_poll = this.stream.poll_next_unpin(cx);
        let main_price = match main_poll {
            Poll::Ready(Some(price)) => price,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        };

        // If there's no conversion stream, return the main price
        if this.conversion_stream.is_none() {
            return Poll::Ready(Some(main_price));
        }

        // Poll the conversion stream
        let conversion_poll = this.conversion_stream.as_mut().unwrap().poll_next_unpin(cx);
        let conversion_price = match conversion_poll {
            Poll::Ready(Some(price)) => price,
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        };

        // Divide main price by conversion price
        // Practically this will be [USDT / BASE] * [USDC / USDT] = USDC / BASE
        let converted_price = main_price * conversion_price;
        Poll::Ready(Some(converted_price))
    }
}

impl PriceStream {
    /// Create a new price stream
    pub fn new(stream: SinglePriceStream) -> Self {
        Self { stream, conversion_stream: None }
    }

    /// Create a new price stream with a conversion stream
    pub fn new_with_conversion(
        stream: SinglePriceStream,
        conversion_stream: SinglePriceStream,
    ) -> Self {
        Self { stream, conversion_stream: Some(conversion_stream) }
    }
}

/// A type alias for a mapped stream prices, indexed by the (source, base,
/// quote) tuple
pub type PriceStreamMap = StreamMap<PriceTopic, PriceStream>;

/// A type alias for a websocket write stream
pub type WsWriteStream = SplitSink<WebSocketStream<TcpStream>, Message>;

/// A type alias for the sender end of a price stream closure channel
pub type ClosureSender = UnboundedSender<Result<(), ServerError>>;

/// A type alias for URL parameters
pub type UrlParams = HashMap<String, String>;

/// A type alias for a router which matches URLs to handlers
pub type HttpRouter = Router<Box<dyn Handler>>;

/// A message that is sent by the price reporter to the client indicating
/// a price update for the given topic
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
    /// The chains to use for token remapping
    pub chains: Vec<Chain>,
    /// The configuration options that may be used by exchange connections
    pub exchange_conn_config: ExchangeConnectionsConfig,
    /// The HMAC key for the admin API. If one is not provided, the admin API
    /// will be disabled.
    pub admin_key: Option<HmacKey>,
    /// Exchanges for which to disable price reporting
    pub disabled_exchanges: Vec<Exchange>,
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
    let chains = match env::var(CHAIN_ID_ENV_VAR) {
        Err(_) => vec![DEFAULT_CHAIN],
        Ok(c) => {
            c.split(',').map(Chain::from_str).collect::<Result<Vec<_>, _>>().unwrap_or_default()
        },
    };
    let coinbase_key_name = env::var(CB_API_KEY_ENV_VAR).ok();
    let coinbase_key_secret = env::var(CB_API_SECRET_ENV_VAR).ok();
    let eth_websocket_addr = env::var(ETH_WS_ADDR_ENV_VAR).ok();
    let admin_key = env::var(ADMIN_KEY_ENV_VAR)
        .ok()
        .map(|key_str| HmacKey::from_base64_string(&key_str).expect("Invalid admin HMAC key"));

    let disabled_exchanges = match env::var(DISABLED_EXCHANGES_ENV_VAR) {
        Err(_) => vec![],
        Ok(exchanges) => exchanges
            .split(',')
            .map(Exchange::from_str)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default(),
    };

    PriceReporterConfig {
        ws_port,
        http_port,
        token_remap_path,
        chains,
        exchange_conn_config: ExchangeConnectionsConfig {
            coinbase_key_name,
            coinbase_key_secret,
            eth_websocket_addr,
        },
        admin_key,
        disabled_exchanges,
    }
}

/// Get the topic name for a given pair info
pub fn get_price_topic_str(topic: &PriceTopic) -> String {
    format!("{}-{}-{}", topic.0, topic.1, topic.2)
}

/// Whether the exchange requires quote conversion
pub fn requires_quote_conversion(exchange: &Exchange) -> bool {
    // Only Renegade exchange requires quote conversion
    exchange == &Exchange::Renegade
}

/// Get all the topics that are subscribed to in a `PriceStreamMap`
pub fn get_subscribed_topics(subscriptions: &PriceStreamMap) -> Vec<String> {
    subscriptions.keys().map(get_price_topic_str).collect_vec()
}

/// Given an address, search through the token remaps to find the token and
/// chain it belongs to
pub fn get_token_and_chain(addr: &str) -> Option<(Token, Chain)> {
    let addr = addr.to_lowercase();
    let remaps = read_token_remaps();
    for (chain, token_map) in remaps.iter() {
        if token_map.get_by_left(&addr).is_some() {
            return Some((Token::from_addr_on_chain(&addr, *chain), *chain));
        }
    }
    None
}

/// Setup token remaps for all given chains
pub fn setup_all_token_remaps(
    token_remap_path: Option<String>,
    chains: &[Chain],
) -> Result<(), ServerError> {
    match token_remap_path {
        // If a token remap path is provided, but multiple chains are specified,
        // return an error
        Some(_) if chains.len() != 1 => Err(ServerError::TokenRemap(
            "When providing a token remap path, exactly one chain must be specified".to_string(),
        )),
        // If a token remap path is provided, use it
        Some(path) => {
            setup_token_remaps(Some(path), chains[0]).map_err(err_str!(ServerError::TokenRemap))
        },
        // Otherwise, fetch remap from default location
        None => chains.iter().try_for_each(|chain| {
            setup_token_remaps(None, *chain).map_err(err_str!(ServerError::TokenRemap))
        }),
    }
}
