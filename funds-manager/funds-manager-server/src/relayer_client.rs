//! Client code for interacting with a configured relayer

use std::time::Duration;

use base64::engine::{Engine, general_purpose as b64_general_purpose};
use http::{HeaderMap, HeaderValue};
use renegade_api::{
    // http::wallet::RedeemNoteRequest,
    RENEGADE_AUTH_HEADER_NAME,
    RENEGADE_SIG_EXPIRATION_HEADER_NAME,
    auth::create_request_signature,
};
use renegade_types_core::{Chain, HmacKey};
use renegade_util::{err_str, get_current_time_millis};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{error::FundsManagerError, helpers::convert_headers};

/// The amount of time (ms) to declare a wallet signature value for
pub const SIG_EXPIRATION_BUFFER_MS: u64 = 5000;

/// A client for interacting with a configured relayer
#[derive(Clone)]
pub struct RelayerClient {
    /// The base URL of the relayer
    pub base_url: String,
    /// The chain the relayer is targeting
    pub chain: Chain,
}

impl RelayerClient {
    /// Create a new relayer client
    pub fn new(base_url: &str, chain: Chain) -> Self {
        Self { base_url: base_url.to_string(), chain }
    }

    ///// Redeem a note into a wallet
    // pub(crate) async fn redeem_note(
    //&self,
    // req: RedeemNoteRequest,
    // wallet_key: &HmacKey,
    //) -> Result<(), FundsManagerError> {
    // todo!("Implement redeem note")
    //}

    // -----------
    // | Helpers |
    // -----------

    #[allow(unused)]
    /// Post to the relayer URL
    async fn post_relayer<Req, Resp>(
        &self,
        path: &str,
        body: &Req,
    ) -> Result<Resp, FundsManagerError>
    where
        Req: Serialize,
        Resp: for<'de> Deserialize<'de>,
    {
        self.post_relayer_with_headers(path, body, &HeaderMap::new()).await
    }

    #[allow(unused)]
    /// Post to the relayer with wallet auth
    async fn post_relayer_with_auth<Req, Resp>(
        &self,
        path: &str,
        body: &Req,
        wallet_key: &HmacKey,
    ) -> Result<Resp, FundsManagerError>
    where
        Req: Serialize,
        Resp: for<'de> Deserialize<'de>,
    {
        let expiration = Duration::from_millis(SIG_EXPIRATION_BUFFER_MS);
        let body_ser = serde_json::to_vec(body).map_err(err_str!(FundsManagerError::Custom))?;
        let mut headers = HeaderMap::new();

        add_expiring_auth_to_headers(path, &mut headers, &body_ser, wallet_key, expiration);

        self.post_relayer_with_headers(path, body, &headers).await
    }

    #[allow(unused)]
    /// Post to the relayer with given headers
    async fn post_relayer_with_headers<Req, Resp>(
        &self,
        path: &str,
        body: &Req,
        headers: &HeaderMap,
    ) -> Result<Resp, FundsManagerError>
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
            .map_err(err_str!(FundsManagerError::Http))?;

        // Deserialize the response
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap();
            return Err(FundsManagerError::http(format!(
                "Failed to send request: {}, {}",
                status, body
            )));
        }

        resp.json::<Resp>().await.map_err(err_str!(FundsManagerError::Parse))
    }

    #[allow(unused)]
    /// Get from the relayer URL
    async fn get_relayer<Resp>(&self, path: &str) -> Result<Resp, FundsManagerError>
    where
        Resp: for<'de> Deserialize<'de>,
    {
        self.get_relayer_with_headers(path, &HeaderMap::new()).await
    }

    #[allow(unused)]
    /// Get from the relayer URL with wallet auth
    async fn get_relayer_with_auth<Resp>(
        &self,
        path: &str,
        wallet_key: &HmacKey,
    ) -> Result<Resp, FundsManagerError>
    where
        Resp: for<'de> Deserialize<'de>,
    {
        let mut headers = HeaderMap::new();
        let expiration = Duration::from_millis(SIG_EXPIRATION_BUFFER_MS);
        add_expiring_auth_to_headers(
            path,
            &mut headers,
            &[], // body
            wallet_key,
            expiration,
        );

        self.get_relayer_with_headers(path, &headers).await
    }

    #[allow(unused)]
    /// Get from the relayer URL with given headers
    async fn get_relayer_with_headers<Resp>(
        &self,
        path: &str,
        headers: &HeaderMap,
    ) -> Result<Resp, FundsManagerError>
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
            .map_err(err_str!(FundsManagerError::Http))?;

        // Parse the response
        if !resp.status().is_success() {
            return Err(FundsManagerError::http(format!(
                "Failed to get relayer path: {}",
                resp.status()
            )));
        }

        resp.json::<Resp>().await.map_err(err_str!(FundsManagerError::Parse))
    }
}

// -----------
// | Helpers |
// -----------

/// Build a reqwest client
fn reqwest_client() -> Result<Client, FundsManagerError> {
    Client::builder()
        .user_agent("fee-sweeper")
        .build()
        .map_err(|_| FundsManagerError::custom("Failed to create reqwest client"))
}

/// Authenticate a relayer request
///
/// We copy the inner auth logic here because we need to convert the headers
/// to work with the relayer's `http` crate version
fn add_expiring_auth_to_headers(
    path: &str,
    headers: &mut HeaderMap,
    body: &[u8],
    key: &HmacKey,
    expiration: Duration,
) {
    // Add a timestamp
    let expiration_ts = get_current_time_millis() + expiration.as_millis() as u64;
    headers.insert(RENEGADE_SIG_EXPIRATION_HEADER_NAME, expiration_ts.into());

    // Add the signature
    let converted_headers = convert_headers(headers);
    let sig = create_request_signature(path, &converted_headers, body, key);
    let b64_sig = b64_general_purpose::STANDARD_NO_PAD.encode(sig);
    let sig_header = HeaderValue::from_str(&b64_sig).expect("b64 encoding should not fail");
    headers.insert(RENEGADE_AUTH_HEADER_NAME, sig_header);
}
