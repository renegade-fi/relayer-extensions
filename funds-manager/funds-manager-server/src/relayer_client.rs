//! Client code for interacting with a configured relayer

use std::time::Duration;

use alloy::signers::local::PrivateKeySigner;
use base64::engine::{general_purpose as b64_general_purpose, Engine};
use http::{HeaderMap, HeaderValue};
use renegade_api::{
    auth::create_request_signature,
    http::{
        task::{GetTaskStatusResponse, GET_TASK_STATUS_ROUTE},
        wallet::{
            CreateWalletRequest, CreateWalletResponse, FindWalletRequest, FindWalletResponse,
            GetWalletResponse, RedeemNoteRequest, RedeemNoteResponse, WithdrawBalanceRequest,
            WithdrawBalanceResponse, CREATE_WALLET_ROUTE, FIND_WALLET_ROUTE, GET_WALLET_ROUTE,
            REDEEM_NOTE_ROUTE, WITHDRAW_BALANCE_ROUTE,
        },
    },
    types::ApiKeychain,
    RENEGADE_AUTH_HEADER_NAME, RENEGADE_SIG_EXPIRATION_HEADER_NAME,
};
use renegade_common::types::{
    chain::Chain,
    hmac::HmacKey,
    wallet::{
        derivation::{derive_blinder_seed, derive_share_seed, derive_wallet_id},
        keychain::KeyChain,
        Wallet, WalletIdentifier,
    },
};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_biguint;
use renegade_util::{err_str, get_current_time_millis};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::{error::FundsManagerError, helpers::convert_headers};

/// The interval at which to poll relayer task status
const POLL_INTERVAL_MS: u64 = 1000;
/// The amount of time (ms) to declare a wallet signature value for
const SIG_EXPIRATION_BUFFER_MS: u64 = 5000;

/// A client for interacting with a configured relayer
#[derive(Clone)]
pub struct RelayerClient {
    /// The base URL of the relayer
    base_url: String,
    /// The chain the relayer is targeting
    pub chain: Chain,
}

impl RelayerClient {
    /// Create a new relayer client
    pub fn new(base_url: &str, chain: Chain) -> Self {
        Self { base_url: base_url.to_string(), chain }
    }

    // ------------------
    // | Wallet Methods |
    // ------------------

    /// Get the wallet for a given id, looking up the wallet if not initially
    /// found
    pub async fn get_wallet(
        &self,
        wallet_id: WalletIdentifier,
        eth_key: &PrivateKeySigner,
        keychain: KeyChain,
    ) -> Result<GetWalletResponse, FundsManagerError> {
        let mut path = GET_WALLET_ROUTE.to_string();
        path = path.replace(":wallet_id", &wallet_id.to_string());

        let wallet_key = keychain.symmetric_key();

        match self.get_relayer_with_auth::<GetWalletResponse>(&path, &wallet_key).await {
            Ok(resp) => Ok(resp),
            Err(err) => {
                warn!("Failed to get wallet {wallet_id} from relayer: {err}");
                self.lookup_wallet(eth_key, keychain).await?;
                self.get_relayer_with_auth::<GetWalletResponse>(&path, &wallet_key).await
            },
        }
    }

    /// Lookup a wallet in the configured relayer
    async fn lookup_wallet(
        &self,
        eth_key: &PrivateKeySigner,
        keychain: KeyChain,
    ) -> Result<(), FundsManagerError> {
        let path = FIND_WALLET_ROUTE.to_string();
        let wallet_id = derive_wallet_id(eth_key).unwrap();
        let blinder_seed = derive_blinder_seed(eth_key).unwrap();
        let share_seed = derive_share_seed(eth_key).unwrap();
        let wallet_key = keychain.symmetric_key();

        let body = FindWalletRequest {
            wallet_id,
            secret_share_seed: scalar_to_biguint(&share_seed),
            blinder_seed: scalar_to_biguint(&blinder_seed),
            private_keychain: ApiKeychain::from(keychain).private_keys,
        };

        let resp: FindWalletResponse =
            self.post_relayer_with_auth(&path, &body, &wallet_key).await?;
        self.await_relayer_task(resp.task_id).await
    }

    /// Create a new wallet via the configured relayer
    pub(crate) async fn create_new_wallet(
        &self,
        wallet: Wallet,
        blinder_seed: &Scalar,
    ) -> Result<(), FundsManagerError> {
        let body = CreateWalletRequest {
            wallet: wallet.into(),
            blinder_seed: scalar_to_biguint(blinder_seed),
        };

        let resp: CreateWalletResponse = self.post_relayer(CREATE_WALLET_ROUTE, &body).await?;
        self.await_relayer_task(resp.task_id).await
    }

    /// Redeem a note into a wallet
    pub(crate) async fn redeem_note(
        &self,
        wallet_id: WalletIdentifier,
        req: RedeemNoteRequest,
        wallet_key: &HmacKey,
    ) -> Result<(), FundsManagerError> {
        let mut path = REDEEM_NOTE_ROUTE.to_string();
        path = path.replace(":wallet_id", &wallet_id.to_string());

        let resp: RedeemNoteResponse = self.post_relayer_with_auth(&path, &req, wallet_key).await?;
        self.await_relayer_task(resp.task_id).await
    }

    /// Withdraw a balance from a wallet
    pub async fn withdraw_balance(
        &self,
        wallet_id: WalletIdentifier,
        mint: String,
        req: WithdrawBalanceRequest,
        root_key: &HmacKey,
    ) -> Result<(), FundsManagerError> {
        let mut path = WITHDRAW_BALANCE_ROUTE.to_string();
        path = path.replace(":wallet_id", &wallet_id.to_string());
        path = path.replace(":mint", &mint);

        let resp: WithdrawBalanceResponse =
            self.post_relayer_with_auth(&path, &req, root_key).await?;
        self.await_relayer_task(resp.task_id).await
    }

    // -----------
    // | Helpers |
    // -----------

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

    /// Get from the relayer URL
    async fn get_relayer<Resp>(&self, path: &str) -> Result<Resp, FundsManagerError>
    where
        Resp: for<'de> Deserialize<'de>,
    {
        self.get_relayer_with_headers(path, &HeaderMap::new()).await
    }

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

    /// Await a relayer task
    async fn await_relayer_task(&self, task_id: Uuid) -> Result<(), FundsManagerError> {
        let mut path = GET_TASK_STATUS_ROUTE.to_string();
        path = path.replace(":task_id", &task_id.to_string());

        // Enter a polling loop until the task finishes
        let poll_interval = Duration::from_millis(POLL_INTERVAL_MS);
        loop {
            // For now, we assume that an error is a 404 in which case the task has
            // completed
            // TODO: Improve this break condition if it proves problematic
            if self.get_relayer::<GetTaskStatusResponse>(&path).await.is_err() {
                break;
            }

            // Sleep for a bit before polling again
            std::thread::sleep(poll_interval);
        }

        Ok(())
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
