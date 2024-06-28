//! The server that serves wallet compliance information

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use std::sync::Arc;

use clap::Parser;
use compliance_api::{ComplianceCheckResponse, ComplianceStatus};
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool};
use error::ComplianceServerError;
use renegade_util::err_str;
use renegade_util::telemetry::{setup_system_logger, LevelFilter};
use warp::{reply::Json, Filter};

use crate::db::get_compliance_entry;

pub mod db;
pub mod error;
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub mod schema;

/// The type of the connection pool
type ConnectionPool = Arc<Pool<ConnectionManager<PgConnection>>>;

/// The CLI for the compliance server
#[derive(Debug, Clone, Parser)]
#[command(about = "The CLI for the compliance server")]
struct Cli {
    /// The port to listen on
    #[arg(short, long)]
    port: u16,
    /// The Chainalysis API key
    #[arg(long)]
    chainalysis_api_key: String,
    /// The url of the compliance database
    #[arg(long)]
    db_url: String,
}

#[tokio::main]
async fn main() {
    setup_system_logger(LevelFilter::INFO);
    let cli = Cli::parse();

    // Create the connection pool
    let manager = ConnectionManager::<PgConnection>::new(cli.db_url.clone());
    let pool = Pool::builder().build(manager).expect("Failed to create pool");
    let pool = Arc::new(pool);

    // Get compliance information for a wallet
    let chainalysis_key = cli.chainalysis_api_key.clone();
    let compliance_check = warp::get()
        .and(warp::path("v0"))
        .and(warp::path("compliance-check"))
        .and(warp::path::param::<String>()) // wallet_address
        .and_then(move |wallet_address| {
            let key = chainalysis_key.clone();
            let pool = pool.clone();

            async move {
                handle_req(wallet_address, &key, pool).await
            }
        });

    // GET /ping
    let ping = warp::get()
        .and(warp::path("ping"))
        .map(|| warp::reply::with_status("PONG", warp::http::StatusCode::OK));

    let routes = compliance_check.or(ping);
    warp::serve(routes).run(([0, 0, 0, 0], cli.port)).await
}

/// Handle a request for a compliance check
async fn handle_req(
    wallet_address: String,
    chainalysis_api_key: &str,
    pool: ConnectionPool,
) -> Result<Json, warp::Rejection> {
    let compliance_status =
        check_wallet_compliance(wallet_address, chainalysis_api_key, pool).await?;
    let resp = ComplianceCheckResponse { compliance_status };
    Ok(warp::reply::json(&resp))
}

/// Check the compliance of a wallet
async fn check_wallet_compliance(
    wallet_address: String,
    chainalysis_api_key: &str,
    pool: ConnectionPool,
) -> Result<ComplianceStatus, ComplianceServerError> {
    // 1. Check the DB first
    let mut conn = pool.get().map_err(err_str!(ComplianceServerError::Db))?;
    let compliance_entry = get_compliance_entry(&wallet_address, &mut conn)?;
    if let Some(compliance_entry) = compliance_entry {
        return Ok(compliance_entry.compliance_status());
    }

    // 2. If not present, check the chainalysis API
    Ok(ComplianceStatus::Compliant)
}
