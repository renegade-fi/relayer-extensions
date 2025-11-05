//! Helper methods for the auth server

use std::{iter, str::FromStr, time::Duration};

use aes_gcm::{AeadCore, Aes128Gcm, aead::Aead};
use alloy::signers::k256::ecdsa::SigningKey;
use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::{Address, Signature, U256, keccak256};
use base64::{Engine as _, engine::general_purpose};
use contracts_common::constants::NUM_BYTES_SIGNATURE;
use rand::thread_rng;
use renegade_api::http::external_match::SignedExternalQuote;
use renegade_common::types::chain::Chain;
use renegade_common::types::token::{Token, get_all_tokens};
use renegade_config::setup_token_remaps;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use renegade_darkpool_client::DarkpoolClient;
use renegade_darkpool_client::client::DarkpoolClientConfig;
use renegade_util::on_chain::{set_external_match_fee, set_protocol_fee};
use uuid::Uuid;

use crate::Cli;
use crate::error::AuthServerError;

// -------------
// | Constants |
// -------------

/// The nonce size for AES128-GCM
const NONCE_SIZE: usize = 12; // 12 bytes, 96 bits

/// The size of a UUID in bytes
const UUID_SIZE: usize = 16;

// ----------------------
// | Encryption Helpers |
// ----------------------

/// AES encrypt a value
///
/// Returns a base64 encoded string of the format [nonce, ciphertext]
pub fn aes_encrypt(value: &str, key: &Aes128Gcm) -> Result<String, AuthServerError> {
    let mut rng = thread_rng();
    let nonce = Aes128Gcm::generate_nonce(&mut rng);
    let ciphertext = key.encrypt(&nonce, value.as_bytes()).map_err(AuthServerError::encryption)?;

    // Encode the [nonce, ciphertext] as a base64 string
    let digest = [nonce.as_slice(), ciphertext.as_slice()].concat();
    let encoded = general_purpose::STANDARD.encode(digest);
    Ok(encoded)
}

/// AES decrypt a value
///
/// Assumes that the input is a base64 encoded string of the format [nonce,
/// ciphertext]
pub fn aes_decrypt(value: &str, key: &Aes128Gcm) -> Result<String, AuthServerError> {
    let decoded = general_purpose::STANDARD.decode(value).map_err(AuthServerError::decryption)?;
    let (nonce, ciphertext) = decoded.split_at(NONCE_SIZE);

    let plaintext_bytes =
        key.decrypt(nonce.into(), ciphertext).map_err(AuthServerError::decryption)?;

    let plaintext = String::from_utf8(plaintext_bytes).map_err(AuthServerError::decryption)?;
    Ok(plaintext)
}

// ---------------------------
// | Gas Sponsorship Helpers |
// ---------------------------

/// Sign a message using a secp256k1 key, serializing the signature to bytes
pub fn sign_message(
    message: &[u8],
    key: &SigningKey,
) -> Result<[u8; NUM_BYTES_SIGNATURE], AuthServerError> {
    let message_hash = keccak256(message);
    let (k256_sig, recid) =
        key.sign_prehash_recoverable(message_hash.as_ref()).map_err(AuthServerError::signing)?;

    let r: U256 = U256::from_be_bytes(k256_sig.r().to_bytes().into());
    let s: U256 = U256::from_be_bytes(k256_sig.s().to_bytes().into());

    let signature = Signature::new(r, s, recid.is_y_odd());
    let mut sig_bytes = signature.as_bytes();

    // This is necessary because `PrimitiveSignature::as_bytes` encodes the `v`
    // component of the signature in "Electrum" notation, i.e. 27 or 28.
    // However, the contracts expect the `v` component to be 0 or 1.
    sig_bytes[NUM_BYTES_SIGNATURE - 1] -= 27;

    Ok(sig_bytes)
}

// ----------------------
// | Conversion Helpers |
// ----------------------

/// Convert a u64 to a U256
pub const fn u64_to_u256(value: u64) -> U256 {
    U256::from_limbs([value, 0, 0, 0])
}

// ----------------
// | Misc Helpers |
// ----------------

/// Get the function selector from calldata
pub fn get_selector(calldata: &[u8]) -> Result<[u8; 4], AuthServerError> {
    calldata
        .get(0..4)
        .ok_or(AuthServerError::serde("expected selector"))?
        .try_into()
        .map_err(AuthServerError::serde)
}

/// Generate a UUID for a signed quote
pub fn generate_quote_uuid(signed_quote: &SignedExternalQuote) -> Uuid {
    let signature_hash = keccak256(signed_quote.signature.as_bytes());
    let mut uuid_bytes = [0u8; UUID_SIZE];
    uuid_bytes.copy_from_slice(&signature_hash[..UUID_SIZE]);

    Uuid::from_bytes(uuid_bytes)
}

// -----------------
// | Setup Helpers |
// -----------------

/// The interval at which we poll filter updates
const DEFAULT_BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(100);

/// Setup the token mapping
pub async fn setup_token_mapping(args: &Cli) -> Result<(), AuthServerError> {
    let chain_id = args.chain_id;
    let token_remap_file = args.token_remap_file.clone();
    tokio::task::spawn_blocking(move || setup_token_remaps(token_remap_file, chain_id))
        .await
        .unwrap()
        .map_err(AuthServerError::setup)
}

/// Set the external match fees & protocol fee
pub async fn set_external_match_fees(
    darkpool_client: &DarkpoolClient,
) -> Result<(), AuthServerError> {
    let protocol_fee = darkpool_client.get_protocol_fee().await.map_err(AuthServerError::setup)?;
    set_protocol_fee(protocol_fee);

    let tokens: Vec<Token> = get_all_tokens()
        .into_iter()
        .chain(iter::once(Token::from_addr(NATIVE_ASSET_ADDRESS)))
        .collect();

    for token in tokens {
        // Fetch the fee override from the contract
        let addr = token.get_alloy_address();
        let fee =
            darkpool_client.get_external_match_fee(addr).await.map_err(AuthServerError::setup)?;

        // Write the fee into the mapping
        let addr_bigint = token.get_addr_biguint();
        set_external_match_fee(&addr_bigint, fee);
    }

    Ok(())
}

/// Parse the gas sponsor address from the CLI args
pub fn parse_gas_sponsor_address(args: &Cli) -> Result<Address, AuthServerError> {
    parse_address(&args.gas_sponsor_address)
}

/// Parse the malleable match connector address from the CLI args
pub fn parse_malleable_match_connector_address(args: &Cli) -> Result<Address, AuthServerError> {
    parse_address(&args.malleable_match_connector_address)
}

/// Parse an address from a string
pub fn parse_address(s: &str) -> Result<Address, AuthServerError> {
    Address::from_str(s).map_err(AuthServerError::setup)
}

/// Create a darkpool client with the provided configuration
pub fn create_darkpool_client(
    darkpool_address: String,
    chain_id: Chain,
    rpc_url: String,
) -> Result<DarkpoolClient, String> {
    // Create the client
    DarkpoolClient::new(DarkpoolClientConfig {
        darkpool_addr: darkpool_address,
        chain: chain_id,
        rpc_url,
        private_key: PrivateKeySigner::random(),
        block_polling_interval: DEFAULT_BLOCK_POLLING_INTERVAL,
    })
    .map_err(|e| format!("Failed to create darkpool client: {e}"))
}

// ---------
// | Tests |
// ---------

#[cfg(test)]
mod tests {
    use aes_gcm::KeyInit;
    use renegade_common::types::hmac::HmacKey;

    use super::*;

    /// Tests AES encryption and decryption
    #[test]
    fn test_aes_encrypt_decrypt() {
        let mut rng = thread_rng();
        let key = Aes128Gcm::new(&Aes128Gcm::generate_key(&mut rng));
        let value = "test string";

        let encrypted = aes_encrypt(value, &key).unwrap();
        let decrypted = aes_decrypt(&encrypted, &key).unwrap();
        assert_eq!(value, decrypted);
    }

    /// Generate an API secret
    #[test]
    fn test_generate_api_secret() {
        let hmac_key = HmacKey::random();
        let base64_hmac_key = hmac_key.to_base64_string();
        println!("base64 hmac key: {base64_hmac_key}");
    }

    /// Generate a management key
    ///
    /// Useful for local testing
    #[test]
    fn test_generate_management_key() {
        let key = HmacKey::random();
        let encoded = general_purpose::STANDARD.encode(key.0);
        println!("management key: {encoded}");
    }
}
