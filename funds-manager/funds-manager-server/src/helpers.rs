//! Helpers for the funds manager server
#![allow(missing_docs)]

use std::{str::FromStr, time::Duration};

use alloy::{
    providers::{
        fillers::{BlobGasFiller, ChainIdFiller, GasFiller},
        DynProvider, Provider, ProviderBuilder,
    },
    rpc::types::{TransactionReceipt, TransactionRequest},
    sol,
};
use alloy_json_rpc::{ErrorPayload, RpcError};
use aws_config::SdkConfig;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_secretsmanager::client::Client as SecretsManagerClient;
use bigdecimal::{BigDecimal, FromPrimitive, RoundingMode, ToPrimitive};
use rand::Rng;
use renegade_common::types::chain::Chain;
use renegade_util::{err_str, telemetry::helpers::backfill_trace_field};
use tracing::instrument;

use crate::{
    cli::{Environment, BLOCK_POLLING_INTERVAL},
    error::FundsManagerError,
};

/// An annotated constant used to indicate that only one confirmation is
/// required for a transaction
pub(crate) const ONE_CONFIRMATION: u64 = 1;
/// The maximum number of retries for a transaction
const TX_MAX_RETRIES: u32 = 5;
/// The minimum delay between retries
const TX_MIN_DELAY: Duration = Duration::from_millis(500);
/// The maximum delay between retries
const TX_MAX_DELAY: Duration = Duration::from_millis(1000);

/// The error message indicating that a nonce is too low
const ERR_NONCE_TOO_LOW: &str = "nonce too low";

// ---------
// | ERC20 |
// ---------

// The ERC20 interface
sol! {
    #[sol(rpc)]
    contract IERC20 {
        event Transfer(address indexed from, address indexed to, uint256 value);
        function symbol() external view returns (string memory);
        function decimals() external view returns (uint8);
        function balanceOf(address account) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
        function approve(address spender, uint256 value) external returns (bool);
        function transfer(address recipient, uint256 amount) external returns (bool);
    }
}

// ----------------
// | ETH JSON RPC |
// ----------------

/// Build a provider for the given RPC url, using simple nonce management
/// and other sensible defaults
///
/// We do not use the default fillers because we want to use the simple
/// nonce manager, which fetches the most recent nonce for each tx. We
/// use different instantiations of the client throughout the codebase,
/// so the cached nonce manager will get out of sync.
pub fn build_provider(url: &str) -> Result<DynProvider, FundsManagerError> {
    let url = url.parse().map_err(FundsManagerError::parse)?;
    let provider = ProviderBuilder::new()
        .disable_recommended_fillers()
        .with_simple_nonce_management()
        .filler(ChainIdFiller::default())
        .filler(GasFiller)
        .filler(BlobGasFiller)
        .connect_http(url);

    provider.client().set_poll_interval(BLOCK_POLLING_INTERVAL);

    Ok(DynProvider::new(provider))
}

/// Send a transaction, retrying on failure
#[instrument(skip_all)]
pub async fn send_tx_with_retry(
    tx: TransactionRequest,
    client: &DynProvider,
    required_confirmations: u64,
) -> Result<TransactionReceipt, FundsManagerError> {
    for _ in 0..TX_MAX_RETRIES {
        // Send the transaction and check for nonce specific issues
        let pending_tx_res = client.send_transaction(tx.clone()).await;
        if let Err(RpcError::ErrorResp(ErrorPayload { ref message, .. })) = pending_tx_res {
            // If the tx failed for nonce issues, sleep for a randomized delay
            if message.contains(ERR_NONCE_TOO_LOW) {
                let delay = rand::thread_rng().gen_range(TX_MIN_DELAY..TX_MAX_DELAY);
                tokio::time::sleep(delay).await;
                continue;
            }
        };

        // If the handler falls through we wait
        let mut pending_tx = pending_tx_res.map_err(FundsManagerError::on_chain)?;
        pending_tx.set_required_confirmations(required_confirmations);
        let receipt = pending_tx.get_receipt().await.map_err(FundsManagerError::on_chain)?;
        backfill_trace_field("tx_hash", receipt.transaction_hash.to_string());
        return Ok(receipt);
    }

    Err(FundsManagerError::on_chain("Transaction failed after retries"))
}

// -----------------------
// | AWS Secrets Manager |
// -----------------------

/// Get the prefix for a chain-specific secret
pub fn get_secret_prefix(chain: Chain) -> Result<String, FundsManagerError> {
    match chain {
        Chain::ArbitrumOne => Ok("/arbitrum/one".to_string()),
        Chain::ArbitrumSepolia => Ok("/arbitrum/sepolia".to_string()),
        Chain::BaseMainnet => Ok("/base/mainnet".to_string()),
        Chain::BaseSepolia => Ok("/base/mainnet".to_string()),
        _ => Err(FundsManagerError::custom("Unsupported chain")),
    }
}

/// Get a secret from AWS Secrets Manager
pub async fn get_secret(
    secret_name: &str,
    config: &SdkConfig,
) -> Result<String, FundsManagerError> {
    let client = SecretsManagerClient::new(config);
    let response = client
        .get_secret_value()
        .secret_id(secret_name)
        .send()
        .await
        .map_err(err_str!(FundsManagerError::SecretsManager))?;

    let secret = response.secret_string().expect("secret value is empty").to_string();
    Ok(secret)
}

/// Add a Renegade wallet to the secrets manager entry so that it may be
/// recovered later
///
/// Returns the name of the secret
pub async fn create_secrets_manager_entry(
    name: &str,
    value: &str,
    config: &SdkConfig,
) -> Result<(), FundsManagerError> {
    create_secrets_manager_entry_with_description(name, value, config, "").await
}

/// Add a Renegade wallet to the secrets manager entry so that it may be
/// recovered later
///
/// Returns the name of the secret
pub async fn create_secrets_manager_entry_with_description(
    name: &str,
    value: &str,
    config: &SdkConfig,
    description: &str,
) -> Result<(), FundsManagerError> {
    let client = SecretsManagerClient::new(config);
    client
        .create_secret()
        .name(name)
        .secret_string(value)
        .description(description)
        .send()
        .await
        .map_err(err_str!(FundsManagerError::SecretsManager))?;

    Ok(())
}

// ----------
// | AWS S3 |
// ----------

/// Fetch an object from S3
pub async fn fetch_s3_object(
    bucket: &str,
    key: &str,
    config: &SdkConfig,
) -> Result<String, FundsManagerError> {
    let client = S3Client::new(config);

    // Fetch the object from S3
    let resp =
        client.get_object().bucket(bucket).key(key).send().await.map_err(FundsManagerError::s3)?;

    // Aggregate the response stream into bytes
    let data = resp.body.collect().await.map_err(FundsManagerError::s3)?;

    // Convert the bytes to a string
    String::from_utf8(data.into_bytes().to_vec()).map_err(FundsManagerError::parse)
}

// --------
// | Misc |
// --------

/// Convert a chain to its environment-agnostic name
pub fn to_env_agnostic_name(chain: Chain) -> String {
    match chain {
        Chain::ArbitrumOne | Chain::ArbitrumSepolia => "arbitrum".to_string(),
        Chain::BaseMainnet | Chain::BaseSepolia => "base".to_string(),
        _ => chain.to_string(),
    }
}

/// Convert an environment-agnostic name to a `Chain` variant
pub fn from_env_agnostic_name(chain: &str, environment: &Environment) -> Chain {
    let arb_chain = match environment {
        Environment::Mainnet => Chain::ArbitrumOne,
        Environment::Testnet => Chain::ArbitrumSepolia,
    };
    let base_chain = match environment {
        Environment::Mainnet => Chain::BaseMainnet,
        Environment::Testnet => Chain::BaseSepolia,
    };

    match chain {
        "arbitrum" => arb_chain,
        "base" => base_chain,
        _ => Chain::from_str(chain).unwrap(),
    }
}

/// Convert a string to title case
pub fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|w| w.chars().next().unwrap().to_uppercase().to_string() + &w[1..])
        .collect::<Vec<String>>()
        .join(" ")
}

/// Round an f64 value up to the given number of decimal places
pub fn round_up(value: f64, decimals: i64) -> Result<f64, FundsManagerError> {
    let value_bigdecimal = BigDecimal::from_f64(value).ok_or(FundsManagerError::conversion(
        format!("Failed to convert {value} to a bigdecimal"),
    ))?;

    let rounded_value_bigdecimal = value_bigdecimal.with_scale_round(decimals, RoundingMode::Up);

    rounded_value_bigdecimal.to_f64().ok_or(FundsManagerError::conversion(format!(
        "Failed to convert {rounded_value_bigdecimal} to a f64"
    )))
}
