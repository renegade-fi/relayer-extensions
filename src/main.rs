//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::{convert::Infallible, net::SocketAddr};

use config::setup_token_remaps;
use errors::ServerError;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Response, Server, StatusCode};
use server::{handle_connection, GlobalPriceStreams};
use tokio::{net::TcpListener, sync::mpsc::unbounded_channel};
use tracing::{error, info};
use util::err_str;
use utils::{parse_config_env_vars, setup_logging, PriceReporterConfig};

mod errors;
mod server;
mod utils;

/// A health check handler for the server
async fn health_check(_: hyper::Request<Body>) -> Result<Response<Body>, Infallible> {
    Ok(Response::builder().status(StatusCode::OK).body(Body::from("OK")).unwrap())
}

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

    // Set up the health check server
    let make_svc = make_service_fn(|_| async { Ok::<_, Infallible>(service_fn(health_check)) });
    let health_server_addr: SocketAddr = format!("0.0.0.0:{:?}", http_port).parse().unwrap();
    // The health check service is infallible so we don't worry about joining /
    // awaiting it
    tokio::spawn(Server::bind(&health_server_addr).serve(make_svc));

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
