//! Helpers for fee indexing and redemption

use renegade_api::http::price_report::{
    GetPriceReportRequest, GetPriceReportResponse, PRICE_REPORT_ROUTE,
};
use renegade_common::types::{exchange::PriceReporterState, token::Token};
use renegade_util::raw_err_str;
use tracing::warn;

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

    let response = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(raw_err_str!("Failed to fetch price: {}"))?;

    if !response.status().is_success() {
        return Err(format!("Failed to fetch price: {}", response.status()));
    }

    let parsed = response
        .json::<GetPriceReportResponse>()
        .await
        .map_err(raw_err_str!("Failed to parse price: {}"))?;
    match parsed.price_report {
        PriceReporterState::Nominal(report) => Ok(Some(report.price)),
        state => {
            warn!("Price report state: {state:?}");
            Ok(None)
        }
    }
}
