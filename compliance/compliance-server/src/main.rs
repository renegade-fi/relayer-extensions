use clap::Parser;
use compliance_api::{ComplianceCheckResponse, ComplianceStatus};
use error::ComplianceServerError;
use renegade_util::telemetry::{setup_system_logger, LevelFilter};
use warp::{reply::Json, Filter};

pub mod error;

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
}

#[tokio::main]
async fn main() {
    setup_system_logger(LevelFilter::INFO);
    let cli = Cli::parse();

    // Get compliance information for a wallet
    let chainalysis_key = cli.chainalysis_api_key.clone();
    let compliance_check = warp::get()
        .and(warp::path("v0"))
        .and(warp::path("compliance-check"))
        .and(warp::path::param::<String>()) // wallet_address
        .and_then(move |wallet_address| {
            let key = chainalysis_key.clone();
            async move {
                handle_req(wallet_address, &key).await
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
) -> Result<Json, warp::Rejection> {
    let compliance_status = check_wallet_compliance(wallet_address, chainalysis_api_key).await?;
    let resp = ComplianceCheckResponse { compliance_status };
    Ok(warp::reply::json(&resp))
}

/// Check the compliance of a wallet
async fn check_wallet_compliance(
    wallet_address: String,
    chainalysis_api_key: &str,
) -> Result<ComplianceStatus, ComplianceServerError> {
    // 1. Check the DB first

    // 2. If not present, check the chainalysis API

    todo!()
}
