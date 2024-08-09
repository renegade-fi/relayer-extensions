//! Helpers for the funds manager server
#![allow(missing_docs)]

use aws_config::SdkConfig;
use aws_sdk_secretsmanager::client::Client as SecretsManagerClient;
use ethers::contract::abigen;
use renegade_util::err_str;

use crate::error::FundsManagerError;

// ---------
// | ERC20 |
// ---------

// The ERC20 interface
abigen!(
    ERC20,
    r#"[
        function balanceOf(address account) external view returns (uint256)
        function symbol() external view returns (string memory)
        function decimals() external view returns (uint8)
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

// -----------------
// | Serialization |
// -----------------

/// A module for serializing and deserializing addresses as strings
pub(crate) mod address_string_serialization {
    use std::str::FromStr;

    use ethers::types::Address;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize an address to a string
    pub fn serialize<S: Serializer>(address: &Address, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&address.to_string())
    }

    /// Deserialize a string to an address
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Address, D::Error> {
        let s = String::deserialize(d)?;
        Address::from_str(&s).map_err(|_| D::Error::custom("Invalid address"))
    }
}

/// A module for serializing and deserializing U256 as strings
pub(crate) mod u256_string_serialization {
    use ethers::types::U256;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize a U256 to a string
    pub fn serialize<S: Serializer>(value: &U256, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    /// Deserialize a string to a U256
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<U256, D::Error> {
        let s = String::deserialize(d)?;
        U256::from_dec_str(&s).map_err(|_| D::Error::custom("Invalid U256 value"))
    }
}

/// A module for serializing and deserializing bytes from a hex string
pub(crate) mod bytes_string_serialization {
    use ethers::types::Bytes;
    use hex::FromHex;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    /// Serialize bytes to a hex string
    pub fn serialize<S: Serializer>(value: &Bytes, s: S) -> Result<S::Ok, S::Error> {
        let hex = format!("{value:#x}");
        s.serialize_str(&hex)
    }

    /// Deserialize a hex string to bytes
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Bytes, D::Error> {
        let s = String::deserialize(d)?;
        Bytes::from_hex(s).map_err(|_| D::Error::custom("Invalid bytes value"))
    }
}

#[cfg(test)]
mod tests {
    use ethers::types::{Address, Bytes, U256};
    use rand::{thread_rng, Rng};

    /// Test serialization and deserialization of an address
    #[test]
    fn test_address_serialization() {
        let addr = Address::random();
        let serialized = serde_json::to_string(&addr).unwrap();
        let deserialized: Address = serde_json::from_str(&serialized).unwrap();
        assert_eq!(addr, deserialized);
    }

    /// Test serialization and deserialization of a U256
    #[test]
    fn test_u256_serialization() {
        let mut rng = thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes);
        let value = U256::from(bytes);

        let serialized = serde_json::to_string(&value).unwrap();
        let deserialized: U256 = serde_json::from_str(&serialized).unwrap();
        assert_eq!(value, deserialized);
    }

    /// Test serialization and deserialization of bytes
    #[test]
    fn test_bytes_serialization() {
        const N: usize = 32;
        let mut rng = thread_rng();
        let bytes: Bytes = (0..N).map(|_| rng.gen_range(0..=u8::MAX)).collect();

        let serialized = serde_json::to_string(&bytes).unwrap();
        let deserialized: Bytes = serde_json::from_str(&serialized).unwrap();
        assert_eq!(bytes, deserialized);
    }
}
