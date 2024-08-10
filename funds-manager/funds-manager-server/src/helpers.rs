//! Helpers for the funds manager server
#![allow(missing_docs)]

use aws_config::SdkConfig;
use aws_sdk_secretsmanager::client::Client as SecretsManagerClient;
use ethers::{contract::abigen, types::H256};
use renegade_util::err_str;

use crate::error::FundsManagerError;

/// A readable type alias for a transaction hash
pub type TransactionHash = H256;

// ---------
// | ERC20 |
// ---------

// The ERC20 interface
abigen!(
    ERC20,
    r#"[
        function symbol() external view returns (string memory)
        function decimals() external view returns (uint8)
        function balanceOf(address account) external view returns (uint256)
        function allowance(address owner, address spender) external view returns (uint256)
        function approve(address spender, uint256 value) external returns (bool)
        function transfer(address recipient, uint256 amount) external returns (bool)
    ]"#
);

// -----------------------
// | AWS Secrets Manager |
// -----------------------

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
