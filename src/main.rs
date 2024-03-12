//! The price reporter is a service that streams midpoint prices for given
//! (price source, base asset, quote asset) tuples over websocket connections.

#![deny(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::net::SocketAddr;

use arbitrum_client::constants::Chain;
use errors::ServerError;
use server::{handle_connection, init_price_streams};
use tokio::net::TcpListener;
use util::err_str;
use config::setup_token_remaps;

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
        setup_token_remaps(None /* remap_file */, Chain::Testnet).map_err(err_str!(ServerError::TokenRemap))
    }).await.unwrap()?;

    init_price_streams();

    // Bind the server to the given port
    let addr: SocketAddr = format!("0.0.0.0:{:?}", PORT).parse().unwrap();

    let listener =
        TcpListener::bind(addr).await.map_err(err_str!(ServerError::WebsocketConnection))?;

    // Await incoming websocket connections
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_connection(stream));
    }

    Ok(())
}
