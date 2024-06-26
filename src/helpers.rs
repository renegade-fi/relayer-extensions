//! Helpers for fee indexing and redemption

use std::time::Duration;

use renegade_api::http::{
    price_report::{GetPriceReportRequest, GetPriceReportResponse, PRICE_REPORT_ROUTE},
    task::{GetTaskStatusResponse, GET_TASK_STATUS_ROUTE},
    wallet::{CreateWalletRequest, CreateWalletResponse, CREATE_WALLET_ROUTE},
};
use renegade_common::types::{exchange::PriceReporterState, token::Token, wallet::Wallet};
use renegade_util::raw_err_str;
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

/// The interval at which to poll relayer task status
const POLL_INTERVAL_MS: u64 = 1000;

/// Get the price for a given mint
pub async fn get_binance_price(
    mint: &str,
    usdc_mint: &str,
    relayer_url: &str,
) -> Result<Option<f64>, String> {
    if mint == usdc_mint {
        return Ok(Some(1.0));
    }

    let url = format!("{}{}", relayer_url, PRICE_REPORT_ROUTE);
    let body = GetPriceReportRequest {
        base_token: Token::from_addr(mint),
        quote_token: Token::from_addr(usdc_mint),
    };
    let response: GetPriceReportResponse = post_relayer(&url, &body).await?;

    match response.price_report {
        PriceReporterState::Nominal(report) => Ok(Some(report.price)),
        state => {
            warn!("Price report state: {state:?}");
            Ok(None)
        }
    }
}

/// Create a new wallet via the configured relayer
pub(crate) async fn create_new_wallet(wallet: Wallet, relayer_url: &str) -> Result<(), String> {
    let url = format!("{}{}", relayer_url, CREATE_WALLET_ROUTE);
    let body = CreateWalletRequest {
        wallet: wallet.into(),
    };

    let resp: CreateWalletResponse = post_relayer(&url, &body).await?;
    await_relayer_task(resp.task_id, relayer_url).await
}

/// Post to the relayer URL
async fn post_relayer<T, U>(url: &str, body: &T) -> Result<U, String>
where
    T: Serialize,
    U: for<'de> Deserialize<'de>,
{
    // Send the request
    let response = reqwest::Client::new()
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(raw_err_str!("Failed to send request: {}"))?;

    if !response.status().is_success() {
        return Err(format!("Failed to send request: {}", response.status()));
    }

    // Deserialize the response
    response
        .json()
        .await
        .map_err(raw_err_str!("Failed to parse response: {}"))
}

/// Get from the relayer URL
async fn get_relayer<T>(path: &str, relayer_url: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let url = format!("{}{}", relayer_url, path);
    let resp = reqwest::get(url)
        .await
        .map_err(raw_err_str!("Failed to get relayer path: {}"))?;

    resp.json::<T>()
        .await
        .map_err(raw_err_str!("Failed to parse response: {}"))
}

/// Await a relayer task
async fn await_relayer_task(task_id: Uuid, relayer_url: &str) -> Result<(), String> {
    let mut path = GET_TASK_STATUS_ROUTE.to_string();
    path = path.replace(":task_id", &task_id.to_string());

    // Enter a polling loop until the task finishes
    let poll_interval = Duration::from_millis(POLL_INTERVAL_MS);
    loop {
        // For now, we assume that an error is a 404 in which case the task has completed
        // TODO: Improve this break condition if it proves problematic
        if get_relayer::<GetTaskStatusResponse>(&path, relayer_url)
            .await
            .is_err()
        {
            break;
        }

        // Sleep for a bit before polling again
        std::thread::sleep(poll_interval);
    }

    Ok(())
}
