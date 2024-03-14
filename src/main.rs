//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::net::SocketAddr;

use arbitrum_client::constants::Chain;
use config::setup_token_remaps;
use errors::ServerError;
use price_reporter::worker::ExchangeConnectionsConfig;
use server::{handle_connection, Server};
use tokio::net::TcpListener;
use util::err_str;

mod errors;
mod server;
mod utils;

/// The port on which the server listens for
/// incoming connections
const PORT: u16 = 4000;

#[tokio::main]
async fn main() -> Result<(), ServerError> {
    // Set up the token remapping
    tokio::task::spawn_blocking(|| {
        // TODO: Accept some minimal config that either allows for a
        // remap file, or specifiying which chain to use
        setup_token_remaps(None /* remap_file */, Chain::Testnet)
            .map_err(err_str!(ServerError::TokenRemap))
    })
    .await
    .unwrap()?;

    let server = Server::new(ExchangeConnectionsConfig::default()).await?;

    // Bind the server to the given port
    let addr: SocketAddr = format!("0.0.0.0:{:?}", PORT).parse().unwrap();

    let listener =
        TcpListener::bind(addr).await.map_err(err_str!(ServerError::WebsocketConnection))?;

    loop {
        tokio::select! {
            // Handle incoming connections
            Ok((stream, _)) = listener.accept() => {
                tokio::spawn(handle_connection(
                    stream,
                    server.global_price_streams.clone(),
                    server.config.clone(),
                ));
            }
            // Handle stream failures
            Some(res) = listen_for_stream_failures(&server) => {
                if res.is_err() {
                    server.global_price_streams.stream_handles.write().await.shutdown().await;
                    return res;
                }
            }
        }
    }
}

/// Await the next stream task to be joined, which only happens
/// in the case of a failure
async fn listen_for_stream_failures(server: &Server) -> Option<Result<(), ServerError>> {
    let mut stream_handles = server.global_price_streams.stream_handles.write().await;
    stream_handles.join_next().await.map(|r| r.map_err(ServerError::JoinError).and_then(|r| r))
}
