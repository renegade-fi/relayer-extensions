//! Helper methods for the auth server

use aes_gcm::{
    aead::{Aead, KeyInit},
    AeadCore, Aes128Gcm,
};
use alloy_primitives::{Address, Bytes, Parity, Signature, U256};
use base64::{engine::general_purpose, Engine as _};
use contracts_common::constants::NUM_BYTES_SIGNATURE;
use ethers::{core::k256::ecdsa::SigningKey, utils::keccak256};
use rand::{thread_rng, Rng};
use serde_json::json;
use warp::reply::Reply;

use crate::error::AuthServerError;

// -------------
// | Constants |
// -------------

/// The gas estimation to use if fetching a gas estimation fails
/// From https://github.com/renegade-fi/renegade/blob/main/workers/api-server/src/http/external_match.rs/#L62
pub const DEFAULT_GAS_ESTIMATION: u64 = 4_000_000; // 4m

/// The nonce size for AES128-GCM
const NONCE_SIZE: usize = 12; // 12 bytes, 96 bits

/// Construct empty json reply
pub fn empty_json_reply() -> impl Reply {
    warp::reply::json(&json!({}))
}

/// AES encrypt a value
///
/// Returns a base64 encoded string of the format [nonce, ciphertext]
pub fn aes_encrypt(value: &str, key: &[u8]) -> Result<String, AuthServerError> {
    let mut rng = thread_rng();
    let cipher = Aes128Gcm::new_from_slice(key).map_err(AuthServerError::encryption)?;
    let nonce = Aes128Gcm::generate_nonce(&mut rng);
    let ciphertext =
        cipher.encrypt(&nonce, value.as_bytes()).map_err(AuthServerError::encryption)?;

    // Encode the [nonce, ciphertext] as a base64 string
    let digest = [nonce.as_slice(), ciphertext.as_slice()].concat();
    let encoded = general_purpose::STANDARD.encode(digest);
    Ok(encoded)
}

/// AES decrypt a value
///
/// Assumes that the input is a base64 encoded string of the format [nonce,
/// ciphertext]
pub fn aes_decrypt(value: &str, key: &[u8]) -> Result<String, AuthServerError> {
    let decoded = general_purpose::STANDARD.decode(value).map_err(AuthServerError::decryption)?;
    let (nonce, ciphertext) = decoded.split_at(NONCE_SIZE);

    let cipher = Aes128Gcm::new_from_slice(key).map_err(AuthServerError::decryption)?;
    let plaintext_bytes =
        cipher.decrypt(nonce.into(), ciphertext).map_err(AuthServerError::decryption)?;
    let plaintext = String::from_utf8(plaintext_bytes).map_err(AuthServerError::decryption)?;
    Ok(plaintext)
}

/// Generate a random nonce for gas sponsorship, signing it and the provided
/// refund address
pub fn gen_signed_sponsorship_nonce(
    refund_address: Address,
    gas_sponsor_auth_key: &SigningKey,
) -> Result<(U256, Bytes), AuthServerError> {
    // Generate a random sponsorship nonce
    let mut nonce_bytes = [0u8; U256::BYTES];
    thread_rng().fill(&mut nonce_bytes);

    // Generate a signature over the nonce + refund address using the gas sponsor
    // key
    let mut message = [0_u8; U256::BYTES + Address::len_bytes()];
    message[..U256::BYTES].copy_from_slice(&nonce_bytes);
    message[U256::BYTES..].copy_from_slice(refund_address.as_ref());

    let signature = sign_message(&message, gas_sponsor_auth_key)?.into();
    let nonce = U256::from_be_bytes(nonce_bytes);

    Ok((nonce, signature))
}

/// Sign a message using a secp256k1 key, serializing the signature to bytes
pub fn sign_message(
    message: &[u8],
    key: &SigningKey,
) -> Result<[u8; NUM_BYTES_SIGNATURE], AuthServerError> {
    let message_hash = keccak256(message);
    let (k256_sig, recid) =
        key.sign_prehash_recoverable(&message_hash).map_err(AuthServerError::signing)?;

    let parity = Parity::Eip155(recid.to_byte() as u64);

    let signature =
        Signature::from_signature_and_parity(k256_sig, parity).map_err(AuthServerError::signing)?;

    Ok(signature.as_bytes())
}

/// Get the function selector from calldata
pub fn get_selector(calldata: &[u8]) -> Result<[u8; 4], AuthServerError> {
    calldata
        .get(0..4)
        .ok_or(AuthServerError::serde("expected selector"))?
        .try_into()
        .map_err(AuthServerError::serde)
}

#[cfg(test)]
mod tests {
    use renegade_common::types::wallet::keychain::HmacKey;

    use super::*;

    /// Tests AES encryption and decryption
    #[test]
    fn test_aes_encrypt_decrypt() {
        let mut rng = thread_rng();
        let key = Aes128Gcm::generate_key(&mut rng);
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
