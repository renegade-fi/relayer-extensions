//! Client code for interacting with a configured relayer
use http::HeaderMap;
use renegade_api::http::price_report::{
    GetPriceReportRequest, GetPriceReportResponse, PRICE_REPORT_ROUTE,
};
use renegade_common::types::{exchange::PriceReporterState, token::Token};
// use renegade_constants::Scalar;
use renegade_util::err_str;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::error::AuthServerError;

/// A client for interacting with a configured relayer
#[derive(Clone)]
pub struct RelayerClient {
    /// The base URL of the relayer
    base_url: String,
}

impl RelayerClient {
    /// Create a new relayer client
    pub fn new(base_url: &str) -> Self {
        Self { base_url: base_url.to_string() }
    }

    /// Get the price for a given mint
    pub async fn get_binance_price(
        &self,
        base_mint: &str,
        quote_mint: &str,
    ) -> Result<Option<f64>, AuthServerError> {
        let body = GetPriceReportRequest {
            base_token: Token::from_addr(base_mint),
            quote_token: Token::from_addr(quote_mint),
        };
        let response: GetPriceReportResponse = self.post_relayer(PRICE_REPORT_ROUTE, &body).await?;

        match response.price_report {
            PriceReporterState::Nominal(report) => Ok(Some(report.price)),
            state => {
                warn!("Price report state: {state:?}");
                Ok(None)
            },
        }
    }

    // -----------
    // | Helpers |
    // -----------

    /// Post to the relayer URL
    async fn post_relayer<Req, Resp>(&self, path: &str, body: &Req) -> Result<Resp, AuthServerError>
    where
        Req: Serialize,
        Resp: for<'de> Deserialize<'de>,
    {
        self.post_relayer_with_headers(path, body, &HeaderMap::new()).await
    }

    /// Post to the relayer with given headers
    async fn post_relayer_with_headers<Req, Resp>(
        &self,
        path: &str,
        body: &Req,
        headers: &HeaderMap,
    ) -> Result<Resp, AuthServerError>
    where
        Req: Serialize,
        Resp: for<'de> Deserialize<'de>,
    {
        // Send a request
        let client = reqwest_client()?;
        let route = format!("{}{}", self.base_url, path);
        let resp = client
            .post(route)
            .json(body)
            .headers(headers.clone())
            .send()
            .await
            .map_err(err_str!(AuthServerError::Http))?;

        // Deserialize the response
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap();
            return Err(AuthServerError::Http(format!(
                "Failed to send request: {}, {}",
                status, body
            )));
        }

        resp.json::<Resp>().await.map_err(err_str!(AuthServerError::Parse))
    }

    /// Get from the relayer URL
    async fn get_relayer<Resp>(&self, path: &str) -> Result<Resp, AuthServerError>
    where
        Resp: for<'de> Deserialize<'de>,
    {
        self.get_relayer_with_headers(path, &HeaderMap::new()).await
    }

    /// Get from the relayer URL with given headers
    async fn get_relayer_with_headers<Resp>(
        &self,
        path: &str,
        headers: &HeaderMap,
    ) -> Result<Resp, AuthServerError>
    where
        Resp: for<'de> Deserialize<'de>,
    {
        let client = reqwest_client()?;
        let url = format!("{}{}", self.base_url, path);
        let resp = client
            .get(url)
            .headers(headers.clone())
            .send()
            .await
            .map_err(err_str!(AuthServerError::Http))?;

        // Parse the response
        if !resp.status().is_success() {
            return Err(AuthServerError::Http(format!(
                "Failed to get relayer path: {}",
                resp.status()
            )));
        }

        resp.json::<Resp>().await.map_err(err_str!(AuthServerError::Parse))
    }
}

// -----------
// | Helpers |
// -----------

/// Build a reqwest client
fn reqwest_client() -> Result<Client, AuthServerError> {
    Client::builder()
        .user_agent("fee-sweeper")
        .build()
        .map_err(|_| AuthServerError::Custom("Failed to create reqwest client".to_string()))
}
