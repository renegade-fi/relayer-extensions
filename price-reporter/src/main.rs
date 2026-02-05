//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(let_chains)]

use std::{collections::HashSet, net::SocketAddr};

use clap::Parser;
use errors::ServerError;
use http_server::HttpServer;
use itertools::Itertools;
use renegade_common::types::{
    exchange::Exchange,
    token::{Token, default_exchange_stable, get_all_base_tokens},
};
use renegade_util::err_str;
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing::{error, info};
use utils::{PairInfo, setup_all_token_remaps};
use ws_server::handle_connection;

use crate::{
    cli::Cli, exchanges::ExchangeConnectionsConfig, price_stream_manager::GlobalPriceStreams,
};

mod cli;
mod errors;
mod exchanges;
mod http_server;
mod price_stream_manager;
mod utils;
mod ws_server;

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    let cli = Cli::parse();

    // Configure telemetry
    cli.configure_telemetry()?;

    // Parse configuration env vars
    let price_reporter_config = cli.parse_price_reporter_config()?;

    // Set up the token remapping
    let token_remap_path = price_reporter_config.token_remap_path.clone();
    let chains = price_reporter_config.chains.clone();
    tokio::task::spawn_blocking(move || {
        setup_all_token_remaps(token_remap_path, &chains).map_err(err_str!(ServerError::TokenRemap))
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
pub(crate) fn init_default_price_streams(
    global_price_streams: &GlobalPriceStreams,
    config: &ExchangeConnectionsConfig,
    disabled_exchanges: Vec<Exchange>,
) -> Result<(), ServerError> {
    info!("Initializing default price streams");

    let disabled_exchanges_set: HashSet<Exchange> = disabled_exchanges.into_iter().collect();
    let enabled_exchanges =
        Exchange::all().into_iter().filter(|e| !disabled_exchanges_set.contains(e)).collect_vec();

    // Collect all streams to initialize; stores tuples (base_token, exchange)
    // Use a hashset to avoid duplicates
    let mut streams = HashSet::new();

    // 1. Add all quote conversion streams
    // These are streams of USDC/DEFAULT_STABLE for each exchange
    let usdc = Token::usdc();
    for exchange in enabled_exchanges.iter().copied() {
        streams.insert((usdc.clone(), exchange));
    }

    // 2. Add in unit streams for all default stables
    // We use unit streams to allow for conversion into stable-stable pairs
    // E.g. USDT/USDC
    for exchange in enabled_exchanges.iter().copied() {
        let default_stable = default_exchange_stable(&exchange);
        streams.insert((default_stable, exchange));
    }

    // 3. Add in streams for all base tokens
    for base_token in get_all_base_tokens() {
        let supported_exchanges: Vec<Exchange> = get_supported_exchanges(&base_token, config)
            .difference(&disabled_exchanges_set)
            .copied()
            .collect();

        supported_exchanges.into_iter().for_each(|exchange| {
            streams.insert((base_token.clone(), exchange));
        });
    }

    // Initialize all streams
    for (base_token, exchange) in streams {
        init_price_stream(base_token, exchange, global_price_streams, config.clone())?;
    }
    Ok(())
}

/// Spawn a task to initialize a price stream for a given token pair
#[allow(clippy::needless_pass_by_value)]
fn init_price_stream(
    base_token: Token,
    exchange: Exchange,
    global_price_streams: &GlobalPriceStreams,
    config: ExchangeConnectionsConfig,
) -> Result<(), ServerError> {
    // We assume that the exchange has a market between the base token
    // and its default stable token
    let pair_info = PairInfo::new_default_stable(exchange, &base_token.get_addr())?;
    let streams = global_price_streams.clone();
    tokio::spawn(async move {
        if let Err(e) = streams.get_or_create_price_stream(pair_info, config.clone()).await {
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
