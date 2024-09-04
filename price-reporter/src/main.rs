//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::net::SocketAddr;

use errors::ServerError;
use http_server::HttpServer;
use renegade_common::types::{
    exchange::Exchange,
    token::{Token, TOKEN_REMAPS, USDC_TICKER, USDT_TICKER, USD_TICKER},
};
use renegade_config::setup_token_remaps;
use renegade_price_reporter::{manager::get_listing_exchanges, worker::ExchangeConnectionsConfig};
use renegade_util::err_str;
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing::{error, info};
use utils::{parse_config_env_vars, setup_logging, PriceReporterConfig};
use ws_server::{handle_connection, GlobalPriceStreams};

mod errors;
mod http_server;
mod utils;
mod ws_server;

/// The default stable to initiate price streams on
const DEFAULT_STABLE: &str = USDT_TICKER;

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Set up logging
    setup_logging();

    // Parse configuration env vars
    let PriceReporterConfig {
        ws_port,
        http_port,
        token_remap_path,
        remap_chain,
        exchange_conn_config,
    } = parse_config_env_vars();

    // Set up the token remapping
    tokio::task::spawn_blocking(move || {
        setup_token_remaps(token_remap_path, remap_chain).map_err(err_str!(ServerError::TokenRemap))
    })
    .await
    .unwrap()?;

    let (closure_tx, mut closure_rx) = unbounded_channel();
    let global_price_streams = GlobalPriceStreams::new(closure_tx);
    init_default_price_streams(&global_price_streams, exchange_conn_config.clone()).await?;

    // Bind the server to the given port
    let addr: SocketAddr = format!("0.0.0.0:{:?}", ws_port).parse().unwrap();

    let listener =
        TcpListener::bind(addr).await.map_err(err_str!(ServerError::WebsocketConnection))?;

    info!("Listening on: {}", addr);

    let http_server =
        HttpServer::new(http_port, exchange_conn_config.clone(), global_price_streams.clone());
    tokio::spawn(http_server.execution_loop());
    // TODO: Handle shutdown of the HTTP server

    loop {
        tokio::select! {
            // Handle incoming connections
            Ok((stream, _)) = listener.accept() => {
                tokio::spawn(handle_connection(
                    stream,
                    global_price_streams.clone(),
                    exchange_conn_config.clone(),
                ));
            }
            // Handle price stream closure
            Some(res) = closure_rx.recv() => {
                if let Err(e) = res {
                    error!("Shutting down server due to error: {}", e);
                    break Ok(());
                }
            }
        }
    }
}

/// Initialize price streams for all default token mapped pairs
async fn init_default_price_streams(
    global_price_streams: &GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
) -> Result<(), ServerError> {
    info!("Initializing default price streams");

    // Get the default token remap
    let quote_token = Token::from_ticker(DEFAULT_STABLE);
    let remap = TOKEN_REMAPS.get().unwrap();
    for (addr, ticker) in remap.clone().into_iter() {
        // Skip stables
        if [USD_TICKER, USDC_TICKER, USDT_TICKER].contains(&ticker.as_str()) {
            continue;
        }

        let base_token = Token::from_addr(&addr);
        let supported_exchanges = get_supported_exchanges(&base_token, &quote_token, &config);
        for exchange in supported_exchanges.into_iter() {
            init_price_stream(
                base_token.clone(),
                quote_token.clone(),
                exchange,
                global_price_streams,
                config.clone(),
            )?;
        }
    }

    Ok(())
}

/// Spawn a task to initialize a price stream for a given token pair
#[allow(clippy::needless_pass_by_value)]
fn init_price_stream(
    base_token: Token,
    quote_token: Token,
    exchange: Exchange,
    global_price_streams: &GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
) -> Result<(), ServerError> {
    let pair_info = (exchange, base_token.clone(), quote_token.clone());
    let mut streams = global_price_streams.clone();
    tokio::spawn(async move {
        if let Err(e) = streams.get_or_create_price_stream(pair_info.clone(), config.clone()).await
        {
            let ticker = base_token.get_ticker().expect("Failed to get ticker");
            error!("Error initializing price stream for {ticker}: {e}");
        }
    });

    Ok(())
}

/// Get the listing exchanges for a given pair
fn get_supported_exchanges(
    base_token: &Token,
    quote_token: &Token,
    config: &ExchangeConnectionsConfig,
) -> Vec<Exchange> {
    let mut supported_exchanges = get_listing_exchanges(base_token, quote_token);
    if config.coinbase_api_key.is_none() || config.coinbase_api_secret.is_none() {
        supported_exchanges.retain(|e| e != &Exchange::Coinbase);
    }

    if config.eth_websocket_addr.is_none() {
        supported_exchanges.retain(|e| e != &Exchange::UniswapV3);
    }

    supported_exchanges
}
