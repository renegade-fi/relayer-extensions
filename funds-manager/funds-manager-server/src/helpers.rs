//! Helpers for the funds manager server

use aws_config::SdkConfig;
use aws_sdk_secretsmanager::client::Client as SecretsManagerClient;
use renegade_util::err_str;

use crate::error::FundsManagerError;

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
