//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::net::SocketAddr;

use config::setup_token_remaps;
use errors::ServerError;
use http_server::HttpServer;
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing::{error, info};
use util::err_str;
use utils::{parse_config_env_vars, setup_logging, PriceReporterConfig};
use ws_server::{handle_connection, GlobalPriceStreams};

mod errors;
mod http_server;
mod utils;
mod ws_server;

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

    // Bind the server to the given port
    let addr: SocketAddr = format!("0.0.0.0:{:?}", ws_port).parse().unwrap();

    let listener =
        TcpListener::bind(addr).await.map_err(err_str!(ServerError::WebsocketConnection))?;

    info!("Listening on: {}", addr);

    let http_server = HttpServer::new(http_port, exchange_conn_config.clone(), global_price_streams.clone());
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
