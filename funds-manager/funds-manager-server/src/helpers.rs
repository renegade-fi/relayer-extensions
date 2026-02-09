//! Helpers for the funds manager server
#![allow(missing_docs)]

use std::{str::FromStr, time::Duration};

use alloy::{
    hex,
    providers::{
        fillers::{BlobGasFiller, ChainIdFiller, GasFiller},
        DynProvider, Provider, ProviderBuilder, WsConnect,
    },
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
    sol,
};
use alloy_json_rpc::{ErrorPayload, RpcError};
use alloy_primitives::{utils::format_units, Address, U256};
use alloy_sol_types::SolEvent;
use aws_config::SdkConfig;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_secretsmanager::client::Client as SecretsManagerClient;
use bigdecimal::{BigDecimal, FromPrimitive, RoundingMode, ToPrimitive};
use rand::Rng;
use renegade_types_core::Chain;
use renegade_util::{err_str, telemetry::helpers::backfill_trace_field};
use reqwest::Response;
use serde::Deserialize;
use tracing::{error, info, instrument};

use crate::{cli::Environment, error::FundsManagerError};

/// An annotated constant used to indicate that two confirmations are
/// required for a transaction
pub(crate) const TWO_CONFIRMATIONS: u64 = 2;
/// The maximum number of retries for a transaction
const TX_MAX_RETRIES: u32 = 5;
/// The minimum delay between retries
const TX_MIN_DELAY: Duration = Duration::from_millis(500);
/// The maximum delay between retries
const TX_MAX_DELAY: Duration = Duration::from_millis(1000);

/// The amount to increase an approval by for a swap
///
/// We "over-approve" so that we don't need to re-approve on every swap
const APPROVAL_AMPLIFIER: U256 = U256::from_limbs([4, 0, 0, 0]);

/// The error message indicating that a nonce is too low
const ERR_NONCE_TOO_LOW: &str = "nonce too low";

/// The darkpool address on Arbitrum One
const ARBITRUM_ONE_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x30bd8eab29181f790d7e495786d4b96d7afdc518"));
/// The darkpool address on Arbitrum Sepolia
const ARBITRUM_SEPOLIA_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x9af58f1ff20ab22e819e40b57ffd784d115a9ef5"));
/// The darkpool address on Base Mainnet
const BASE_MAINNET_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0xb4a96068577141749CC8859f586fE29016C935dB"));
/// The darkpool address on Base Sepolia
const BASE_SEPOLIA_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x653C95391644EEE16E4975a7ef1f46e0B8276695"));
/// The darkpool address on Ethereum Mainnet
const ETHEREUM_MAINNET_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000")); // Placeholder
/// The darkpool address on Ethereum Sepolia
const ETHEREUM_SEPOLIA_DARKPOOL_ADDRESS: Address =
    Address::new(hex!("0x45537c28F245645CC1E7F7258FCC18A189CE16e3"));

/// The gas sponsor address on Arbitrum One
const ARBITRUM_ONE_GAS_SPONSOR_ADDRESS: Address =
    Address::new(hex!("0xbacedc261add2e273801b9f64133bb709efbc3d8"));
/// The gas sponsor address on Arbitrum Sepolia
const ARBITRUM_SEPOLIA_GAS_SPONSOR_ADDRESS: Address =
    Address::new(hex!("0xaab1a91fdc6498d1dc1acba9bf7d751cd744653c"));
/// The gas sponsor address on Base Mainnet
const BASE_MAINNET_GAS_SPONSOR_ADDRESS: Address =
    Address::new(hex!("0xE1AD298b51a8924C539d1530E8E5E39232006771"));
/// The gas sponsor address on Base Sepolia
const BASE_SEPOLIA_GAS_SPONSOR_ADDRESS: Address =
    Address::new(hex!("0x2fDB4e70Db12599b04642b3d023E75f6439c5707"));
/// The gas sponsor address on Ethereum Mainnet
const ETHEREUM_MAINNET_GAS_SPONSOR_ADDRESS: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000")); // Placeholder
/// The gas sponsor address on Ethereum Sepolia
const ETHEREUM_SEPOLIA_GAS_SPONSOR_ADDRESS: Address =
    Address::new(hex!("0x8E330790c68b9462123848e418BaDB3399c7D26F"));

/// The v2 gas sponsor address on Arbitrum One
const ARBITRUM_ONE_GAS_SPONSOR_ADDRESS_V2: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));
/// The v2 gas sponsor address on Arbitrum Sepolia
const ARBITRUM_SEPOLIA_GAS_SPONSOR_ADDRESS_V2: Address =
    Address::new(hex!("44183ad1d4ec082e9EEb7e9665211CC35De5123b"));
/// The v2 gas sponsor address on Base Mainnet
const BASE_MAINNET_GAS_SPONSOR_ADDRESS_V2: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000"));
/// The v2 gas sponsor address on Base Sepolia
const BASE_SEPOLIA_GAS_SPONSOR_ADDRESS_V2: Address =
    Address::new(hex!("37f529F812d6D5ABa07BFcb9cB374f2450B782eE"));
/// The v2 gas sponsor address on Ethereum Mainnet
const ETHEREUM_MAINNET_GAS_SPONSOR_ADDRESS_V2: Address =
    Address::new(hex!("0x0000000000000000000000000000000000000000")); // Placeholder
/// The v2 gas sponsor address on Ethereum Sepolia
const ETHEREUM_SEPOLIA_GAS_SPONSOR_ADDRESS_V2: Address =
    Address::new(hex!("0x2fDB4e70Db12599b04642b3d023E75f6439c5707"));

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

/// Construct a base provider w/ a websocket connection to the given RPC URL
pub async fn base_ws_provider(ws_rpc_url: &str) -> Result<DynProvider, FundsManagerError> {
    let ws = WsConnect::new(ws_rpc_url);
    ProviderBuilder::default()
        .connect_ws(ws)
        .await
        .map_err(FundsManagerError::on_chain)
        .map(|provider| provider.erased())
}

/// Build a provider for the given base provider, using simple nonce management
/// and other sensible defaults
///
/// We do not use the default fillers because we want to use the simple
/// nonce manager, which fetches the most recent nonce for each tx. We
/// use different instantiations of the client throughout the codebase,
/// so the cached nonce manager will get out of sync.
pub fn build_provider(base_provider: DynProvider, wallet: Option<PrivateKeySigner>) -> DynProvider {
    let builder = ProviderBuilder::default()
        .with_simple_nonce_management()
        .filler(ChainIdFiller::default())
        .filler(GasFiller)
        .filler(BlobGasFiller::default());

    if let Some(wallet) = wallet {
        builder.wallet(wallet).connect_provider(base_provider).erased()
    } else {
        builder.connect_provider(base_provider).erased()
    }
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

/// Get the erc20 balance of an address, as a U256
pub async fn get_erc20_balance_raw(
    token_address: &str,
    address: &str,
    provider: DynProvider,
) -> Result<U256, FundsManagerError> {
    // Set up the contract instance
    let token_address = Address::from_str(token_address).map_err(FundsManagerError::parse)?;
    let address = Address::from_str(address).map_err(FundsManagerError::parse)?;
    let erc20 = IERC20::new(token_address, provider);

    // Fetch the balance
    erc20.balanceOf(address).call().await.map_err(FundsManagerError::on_chain)
}

/// Get the erc20 balance of an address
pub async fn get_erc20_balance(
    token_address: &str,
    address: &str,
    provider: DynProvider,
) -> Result<f64, FundsManagerError> {
    // Set up the contract instance
    let token_address = Address::from_str(token_address).map_err(FundsManagerError::parse)?;
    let address = Address::from_str(address).map_err(FundsManagerError::parse)?;
    let erc20 = IERC20::new(token_address, provider);

    // Fetch the balance and correct for the ERC20 decimal precision
    let decimals = erc20.decimals().call().await.map_err(FundsManagerError::on_chain)?;
    let balance = erc20.balanceOf(address).call().await.map_err(FundsManagerError::on_chain)?;

    let bal_str = format_units(balance, decimals).map_err(FundsManagerError::parse)?;
    let bal_f64 = bal_str.parse::<f64>().map_err(FundsManagerError::parse)?;

    Ok(bal_f64)
}

/// Approve an erc20 allowance
pub(crate) async fn approve_erc20_allowance(
    token_address: Address,
    spender: Address,
    owner: Address,
    amount: U256,
    rpc_provider: DynProvider,
) -> Result<(), FundsManagerError> {
    let erc20 = IERC20::new(token_address, rpc_provider.clone());

    // First, check if the allowance is already sufficient
    let allowance =
        erc20.allowance(owner, spender).call().await.map_err(FundsManagerError::on_chain)?;

    if allowance >= amount {
        info!("Already approved erc20 allowance for {spender:#x}");
        return Ok(());
    }

    // Otherwise, approve the allowance
    let approval_amount = amount * APPROVAL_AMPLIFIER;
    let tx = erc20.approve(spender, approval_amount).into_transaction_request();
    let receipt = send_tx_with_retry(tx, &rpc_provider, TWO_CONFIRMATIONS).await?;

    info!("Approved erc20 allowance at: {:#x}", receipt.transaction_hash);
    Ok(())
}

/// Compute the gas cost of a transaction in WEI
pub fn get_gas_cost(receipt: &TransactionReceipt) -> U256 {
    U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price)
}

/// Get the amount of a token that was received by a recipient in a transaction
pub fn get_received_amount(
    receipt: &TransactionReceipt,
    token_address: Address,
    recipient: Address,
) -> U256 {
    receipt
        .logs()
        .iter()
        .map(|log| {
            if log.address() != token_address {
                return U256::ZERO;
            }

            let transfer = match IERC20::Transfer::decode_log(&log.inner) {
                Ok(transfer) => transfer,
                // Failure to decode implies the event is not a transfer
                Err(_) => return U256::ZERO,
            };

            if transfer.to == recipient {
                return transfer.value;
            }

            U256::ZERO
        })
        .sum()
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
        Chain::EthereumMainnet => Ok("/ethereum/mainnet".to_string()),
        Chain::EthereumSepolia => Ok("/ethereum/mainnet".to_string()),
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
        Chain::EthereumMainnet | Chain::EthereumSepolia => "ethereum".to_string(),
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
    let ethereum_chain = match environment {
        Environment::Mainnet => Chain::EthereumMainnet,
        Environment::Testnet => Chain::EthereumSepolia,
    };

    match chain {
        "arbitrum" => arb_chain,
        "base" => base_chain,
        "ethereum" => ethereum_chain,
        _ => Chain::from_str(chain).unwrap(),
    }
}

/// Convert a chain to its chain id
pub fn to_chain_id(chain: Chain) -> u64 {
    match chain {
        Chain::ArbitrumOne => 42161,
        Chain::ArbitrumSepolia => 421614,
        Chain::BaseMainnet => 8453,
        Chain::BaseSepolia => 84532,
        Chain::EthereumMainnet => 1,
        Chain::EthereumSepolia => 11155111,
        Chain::Devnet => 0,
    }
}

/// Convert a chain id to a `Chain` variant
pub fn from_chain_id(chain_id: u64) -> Result<Chain, String> {
    match chain_id {
        42161 => Ok(Chain::ArbitrumOne),
        421614 => Ok(Chain::ArbitrumSepolia),
        8453 => Ok(Chain::BaseMainnet),
        84532 => Ok(Chain::BaseSepolia),
        1 => Ok(Chain::EthereumMainnet),
        11155111 => Ok(Chain::EthereumSepolia),
        0 => Ok(Chain::Devnet),
        _ => Err("Invalid chain ID".to_string()),
    }
}

/// Get the darkpool address for a given chain
pub fn get_darkpool_address(chain: Chain) -> Address {
    match chain {
        Chain::ArbitrumOne => ARBITRUM_ONE_DARKPOOL_ADDRESS,
        Chain::ArbitrumSepolia => ARBITRUM_SEPOLIA_DARKPOOL_ADDRESS,
        Chain::BaseMainnet => BASE_MAINNET_DARKPOOL_ADDRESS,
        Chain::BaseSepolia => BASE_SEPOLIA_DARKPOOL_ADDRESS,
        Chain::EthereumMainnet => ETHEREUM_MAINNET_DARKPOOL_ADDRESS,
        Chain::EthereumSepolia => ETHEREUM_SEPOLIA_DARKPOOL_ADDRESS,
        _ => panic!("{}", format!("get_darkpool_address: Invalid chain {}", chain)),
    }
}

/// Get the gas sponsor address for a given chain
pub fn get_gas_sponsor_address(chain: Chain) -> Address {
    match chain {
        Chain::ArbitrumOne => ARBITRUM_ONE_GAS_SPONSOR_ADDRESS,
        Chain::ArbitrumSepolia => ARBITRUM_SEPOLIA_GAS_SPONSOR_ADDRESS,
        Chain::BaseMainnet => BASE_MAINNET_GAS_SPONSOR_ADDRESS,
        Chain::BaseSepolia => BASE_SEPOLIA_GAS_SPONSOR_ADDRESS,
        Chain::EthereumMainnet => ETHEREUM_MAINNET_GAS_SPONSOR_ADDRESS,
        Chain::EthereumSepolia => ETHEREUM_SEPOLIA_GAS_SPONSOR_ADDRESS,
        _ => panic!("{}", format!("get_gas_sponsor_address: Invalid chain {}", chain)),
    }
}

/// Get the v2 gas sponsor address for a given chain
pub fn get_gas_sponsor_address_v2(chain: Chain) -> Address {
    match chain {
        Chain::ArbitrumOne => ARBITRUM_ONE_GAS_SPONSOR_ADDRESS_V2,
        Chain::ArbitrumSepolia => ARBITRUM_SEPOLIA_GAS_SPONSOR_ADDRESS_V2,
        Chain::BaseMainnet => BASE_MAINNET_GAS_SPONSOR_ADDRESS_V2,
        Chain::BaseSepolia => BASE_SEPOLIA_GAS_SPONSOR_ADDRESS_V2,
        Chain::EthereumMainnet => ETHEREUM_MAINNET_GAS_SPONSOR_ADDRESS_V2,
        Chain::EthereumSepolia => ETHEREUM_SEPOLIA_GAS_SPONSOR_ADDRESS_V2,
        _ => panic!("{}", format!("get_gas_sponsor_address_v2: Invalid chain {}", chain)),
    }
}

/// Convert a string to title case
pub fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|w| w.chars().next().unwrap().to_uppercase().to_string() + &w[1..])
        .collect::<Vec<String>>()
        .join(" ")
}

/// Check if a byte slice contains a subslice
pub fn contains_byte_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
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

/// Convert a `warp::hyper::HeaderMap` (using the old `http` crate version)
/// into a fresh `http::HeaderMap` that comes from the new 1.x `http` crate.
///
/// We avoid the `IntoHeaderName` trait conflict that arises when two
/// different `http` crate versions are in the dependency graph by copying
/// header names/values byte‐for‐byte into the new types.
pub fn convert_headers(headers: &warp::hyper::HeaderMap) -> http1::HeaderMap {
    let mut converted = http1::HeaderMap::new();
    for (name, value) in headers.iter() {
        let name_bytes = name.as_ref();
        let name = match http1::header::HeaderName::from_bytes(name_bytes) {
            Ok(n) => n,
            Err(_) => continue, // Skip invalid names
        };

        let value_bytes = value.as_ref();
        let value = match http1::HeaderValue::from_bytes(value_bytes) {
            Ok(v) => v,
            Err(_) => continue, // Skip invalid values
        };
        converted.append(name, value);
    }

    converted
}

/// Handle an HTTP response
pub async fn handle_http_response<Res: for<'de> Deserialize<'de>>(
    response: Response,
) -> Result<Res, FundsManagerError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await?;
        let msg = format!("Unexpected status code: {status}\nbody: {body}");
        error!(msg);
        return Err(FundsManagerError::http(msg));
    }

    response.json::<Res>().await.map_err(FundsManagerError::http)
}
