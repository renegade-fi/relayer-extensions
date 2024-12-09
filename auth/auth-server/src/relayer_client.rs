//! Client code for interacting with a configured relayer
use crate::error::AuthServerError;
use crate::ApiError;
use bytes::Bytes;
use http::{HeaderMap, Method, Response};
use renegade_api::auth::add_expiring_auth_to_headers;
use renegade_api::http::price_report::{
    GetPriceReportRequest, GetPriceReportResponse, PRICE_REPORT_ROUTE,
};
use renegade_common::types::wallet::keychain::HmacKey;
use renegade_common::types::{exchange::PriceReporterState, token::Token};
use renegade_util::err_str;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{error, warn};

/// The duration for which the admin authentication is valid
const ADMIN_AUTH_DURATION_MS: u64 = 5_000; // 5 seconds

/// A client for interacting with a configured relayer
#[derive(Clone)]
pub struct RelayerClient {
    /// The base URL of the relayer
    base_url: String,
    /// The HTTP client
    client: Client,
    /// The admin key for the relayer
    relayer_admin_key: HmacKey,
}

impl RelayerClient {
    /// Create a new relayer client
    pub fn new(base_url: &str, relayer_admin_key: HmacKey) -> Self {
        Self { base_url: base_url.to_string(), client: Client::new(), relayer_admin_key }
    }

    /// Send a proxied request to the relayer with admin authentication
    pub async fn send_admin_request(
        &self,
        method: Method,
        path: &str,
        mut headers: HeaderMap,
        body: Bytes,
    ) -> Result<Response<Bytes>, ApiError> {
        // Admin authenticate the request
        self.admin_authenticate(path, &mut headers, &body)?;

        // Forward the request to the relayer
        let url = format!("{}{}", self.base_url, path);
        let req = self.client.request(method, &url).headers(headers).body(body);
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers().clone();
                let body = resp.bytes().await.map_err(|e| {
                    ApiError::internal(format!("Failed to read response body: {e}"))
                })?;

                let mut response = warp::http::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;

                Ok(response)
            },
            Err(e) => {
                error!("Error proxying request: {}", e);
                Err(ApiError::internal(e))
            },
        }
    }

    /// Admin authenticate a request
    pub fn admin_authenticate(
        &self,
        path: &str,
        headers: &mut HeaderMap,
        body: &[u8],
    ) -> Result<(), ApiError> {
        let expiration = Duration::from_millis(ADMIN_AUTH_DURATION_MS);
        add_expiring_auth_to_headers(path, headers, body, &self.relayer_admin_key, expiration);
        Ok(())
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
        let route = format!("{}{}", self.base_url, path);
        let resp = self
            .client
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
}
