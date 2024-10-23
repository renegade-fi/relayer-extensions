//! Helper methods for the auth server

use aes_gcm::{
    aead::{Aead, KeyInit},
    AeadCore, Aes128Gcm,
};
use base64::{engine::general_purpose, Engine as _};
use rand::thread_rng;
use serde_json::json;
use warp::reply::Reply;

use crate::error::AuthServerError;

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

#[cfg(test)]
mod tests {
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
}
