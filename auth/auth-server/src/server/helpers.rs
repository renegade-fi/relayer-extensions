//! Helper methods for the auth server

use aes_gcm::{
    aead::{Aead, KeyInit},
    AeadCore, Aes128Gcm,
};
use alloy_primitives::{Address, Bytes, Parity, Signature, U256 as AlloyU256};
use base64::{engine::general_purpose, Engine as _};
use bigdecimal::{
    num_bigint::{BigInt, Sign},
    BigDecimal, One,
};
use contracts_common::constants::NUM_BYTES_SIGNATURE;
use ethers::{core::k256::ecdsa::SigningKey, types::U256, utils::keccak256};
use rand::{thread_rng, Rng};
use renegade_api::http::external_match::AtomicMatchApiBundle;
use renegade_common::types::token::Token;
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

/// Generate a random nonce for gas sponsorship, signing it along with
/// the provided refund address, and optionally the conversion rate
pub fn gen_signed_sponsorship_nonce(
    refund_address: Address,
    conversion_rate: Option<AlloyU256>,
    gas_sponsor_auth_key: &SigningKey,
) -> Result<(AlloyU256, Bytes), AuthServerError> {
    // Generate a random sponsorship nonce
    let mut nonce_bytes = [0u8; AlloyU256::BYTES];
    thread_rng().fill(&mut nonce_bytes);

    // Construct & sign the message
    let mut message = Vec::new();
    message.extend_from_slice(&nonce_bytes);
    message.extend_from_slice(refund_address.as_ref());
    if let Some(conversion_rate) = conversion_rate {
        message.extend_from_slice(&conversion_rate.to_be_bytes::<{ AlloyU256::BYTES }>());
    }

    let signature = sign_message(&message, gas_sponsor_auth_key)?.into();
    let nonce = AlloyU256::from_be_bytes(nonce_bytes);

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

/// Convert an ethers U256 to a BigDecimal
pub fn ethers_u256_to_bigdecimal(value: U256) -> BigDecimal {
    let mut value_bytes = [0u8; 32];
    value.to_big_endian(&mut value_bytes);
    let bigint = BigInt::from_bytes_be(Sign::Plus, &value_bytes);
    BigDecimal::from(bigint)
}

/// Get the nominal price of the buy token in USDC,
/// i.e. whole units of USDC per nominal unit of TOKEN
pub fn get_nominal_buy_token_price(
    match_bundle: &AtomicMatchApiBundle,
) -> Result<BigDecimal, AuthServerError> {
    let buy_mint = &match_bundle.receive.mint;
    let quote_mint = &match_bundle.match_result.quote_mint;
    let buying_quote = buy_mint.to_lowercase() == quote_mint.to_lowercase();

    // Compute TOKEN price from match result, in nominal terms
    // (i.e. units of USDC per unit of TOKEN)
    let price = if buying_quote {
        // The quote token is always USDC, so price is 1
        BigDecimal::one()
    } else {
        let base_amount = BigDecimal::from(match_bundle.match_result.base_amount);
        let quote_amount = BigDecimal::from(match_bundle.match_result.quote_amount);
        quote_amount / base_amount
    };

    let quote_decimals = Token::from_addr(quote_mint)
        .get_decimals()
        .ok_or(AuthServerError::custom("quote token has no decimals"))?;

    let adjustment: BigDecimal = BigInt::from(10).pow(quote_decimals as u32).into();

    Ok(price / adjustment)
}

#[cfg(test)]
mod tests {
    use renegade_common::types::hmac::HmacKey;

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
