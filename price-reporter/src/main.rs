//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
};

use errors::ServerError;
use http_server::HttpServer;
use renegade_common::types::{
    exchange::Exchange,
    token::{default_exchange_stable, Token, USDC_TICKER, USDT_TICKER, USD_TICKER},
};
use renegade_price_reporter::worker::ExchangeConnectionsConfig;
use renegade_util::err_str;
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing::{error, info};
use utils::{
    get_all_tokens_filtered, parse_config_env_vars, setup_all_token_remaps, setup_logging,
};
use ws_server::{handle_connection, GlobalPriceStreams};

mod errors;
mod http_server;
mod utils;
mod ws_server;

/// Stablecoin tickers to filter
const STABLECOIN_TICKERS: [&str; 3] = [USD_TICKER, USDC_TICKER, USDT_TICKER];

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Set up logging
    setup_logging();

    // Parse configuration env vars
    let price_reporter_config = parse_config_env_vars();

    // Set up the token remapping
    let remap_chains = price_reporter_config.remap_chains.clone();
    tokio::task::spawn_blocking(move || {
        setup_all_token_remaps(&remap_chains).map_err(err_str!(ServerError::TokenRemap))
    })
    .await
    .unwrap()?;

    let (closure_tx, mut closure_rx) = unbounded_channel();
    let global_price_streams = GlobalPriceStreams::new(closure_tx);
    init_default_price_streams(
        &global_price_streams,
        &price_reporter_config.exchange_conn_config,
        price_reporter_config.disabled_exchanges.clone(),
    )?;

    // Bind the server to the given port
    let addr: SocketAddr = format!("0.0.0.0:{:?}", price_reporter_config.ws_port).parse().unwrap();

    let listener =
        TcpListener::bind(addr).await.map_err(err_str!(ServerError::WebsocketConnection))?;

    info!("Listening on: {}", addr);

    let http_server = HttpServer::new(&price_reporter_config, global_price_streams.clone());
    tokio::spawn(http_server.execution_loop());
    // TODO: Handle shutdown of the HTTP server

    loop {
        tokio::select! {
            // Handle incoming connections
            Ok((stream, _)) = listener.accept() => {
                tokio::spawn(handle_connection(
                    stream,
                    global_price_streams.clone(),
                    price_reporter_config.exchange_conn_config.clone(),
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
pub fn init_default_price_streams(
    global_price_streams: &GlobalPriceStreams,
    config: &ExchangeConnectionsConfig,
    disabled_exchanges: Vec<Exchange>,
) -> Result<(), ServerError> {
    info!("Initializing default price streams");

    let disabled_exchanges_set: HashSet<Exchange> = disabled_exchanges.into_iter().collect();

    // Get tokens with distinct tickers from the token remap
    let distinct_tokens = get_all_tokens_filtered(&STABLECOIN_TICKERS).into_iter().fold(
        HashMap::new(),
        |mut m, token| {
            if let Some(ticker) = token.get_ticker() {
                m.entry(ticker.to_string()).or_insert(token);
            }
            m
        },
    );

    // Iterate over distinct tokens
    for base_token in distinct_tokens.into_values() {
        let supported_exchanges: Vec<Exchange> = get_supported_exchanges(&base_token, config)
            .difference(&disabled_exchanges_set)
            .copied()
            .collect();

        for exchange in supported_exchanges {
            let quote_token = default_exchange_stable(&exchange);
            // We assume that the exchange has a market between the base token
            // and its default stable token
            init_price_stream(
                base_token.clone(),
                quote_token,
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
    let streams = global_price_streams.clone();
    tokio::spawn(async move {
        if let Err(e) = streams.get_or_create_price_stream(pair_info.clone(), config.clone()).await
        {
            let ticker = base_token.get_ticker().expect("Failed to get ticker");
            error!("Error initializing price stream for {ticker}: {e}");
        }
    });

    Ok(())
}

/// Get the listing exchanges for a given base token
fn get_supported_exchanges(
    base_token: &Token,
    config: &ExchangeConnectionsConfig,
) -> HashSet<Exchange> {
    let mut supported_exchanges = base_token.supported_exchanges();
    if !config.coinbase_configured() {
        supported_exchanges.remove(&Exchange::Coinbase);
    }
    if !config.uniswap_v3_configured() {
        supported_exchanges.remove(&Exchange::UniswapV3);
    }

    supported_exchanges
}
