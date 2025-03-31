//! Helper methods for the auth server

use aes_gcm::{
    aead::{Aead, KeyInit},
    AeadCore, Aes128Gcm,
};
use alloy_primitives::{Address, Bytes as AlloyBytes, PrimitiveSignature, U256 as AlloyU256};
use base64::{engine::general_purpose, Engine as _};
use bigdecimal::{
    num_bigint::{BigInt, Sign},
    BigDecimal, One,
};
use bytes::Bytes;
use contracts_common::constants::NUM_BYTES_SIGNATURE;
use ethers::{core::k256::ecdsa::SigningKey, types::U256, utils::keccak256};
use http::{header::CONTENT_LENGTH, Response};
use rand::{thread_rng, Rng};
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_common::types::token::Token;
use serde::Serialize;
use serde_json::json;
use warp::reply::Reply;

use crate::error::AuthServerError;

// -------------
// | Constants |
// -------------

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
/// the provided refund address and the refund amount
pub fn gen_signed_sponsorship_nonce(
    refund_address: Address,
    refund_amount: AlloyU256,
    gas_sponsor_auth_key: &SigningKey,
) -> Result<(AlloyU256, AlloyBytes), AuthServerError> {
    // Generate a random sponsorship nonce
    let mut nonce_bytes = [0u8; AlloyU256::BYTES];
    thread_rng().fill(&mut nonce_bytes);

    // Construct & sign the message
    let mut message = Vec::new();
    message.extend_from_slice(&nonce_bytes);
    message.extend_from_slice(refund_address.as_ref());
    message.extend_from_slice(&refund_amount.to_be_bytes::<{ AlloyU256::BYTES }>());

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

    let r: AlloyU256 = AlloyU256::from_be_bytes(k256_sig.r().to_bytes().into());
    let s: AlloyU256 = AlloyU256::from_be_bytes(k256_sig.s().to_bytes().into());

    let signature = PrimitiveSignature::new(r, s, recid.is_y_odd());
    let mut sig_bytes = signature.as_bytes();

    // This is necessary because `PrimitiveSignature::as_bytes` encodes the `v`
    // component of the signature in "Electrum" notation, i.e. 27 or 28.
    // However, the contracts expect the `v` component to be 0 or 1.
    sig_bytes[NUM_BYTES_SIGNATURE - 1] -= 27;

    Ok(sig_bytes)
}

/// Get the function selector from calldata
pub fn get_selector(calldata: &[u8]) -> Result<[u8; 4], AuthServerError> {
    calldata
        .get(0..4)
        .ok_or(AuthServerError::serde("expected selector"))?
        .try_into()
        .map_err(AuthServerError::serde)
}

/// Convert an ethers U256 to an alloy U256
pub fn ethers_u256_to_alloy_u256(value: U256) -> AlloyU256 {
    let mut value_bytes = [0_u8; 32];
    value.to_big_endian(&mut value_bytes);
    AlloyU256::from_be_bytes(value_bytes)
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
    buy_mint: &str,
    match_result: &ApiExternalMatchResult,
) -> Result<BigDecimal, AuthServerError> {
    let quote_mint = &match_result.quote_mint;
    let buying_quote = buy_mint.to_lowercase() == quote_mint.to_lowercase();

    // Compute TOKEN price from match result, in nominal terms
    // (i.e. units of USDC per unit of TOKEN)
    let price = if buying_quote {
        // The quote token is always USDC, so price is 1
        BigDecimal::one()
    } else {
        let base_amount = BigDecimal::from(match_result.base_amount);
        let quote_amount = BigDecimal::from(match_result.quote_amount);
        quote_amount / base_amount
    };

    let quote_decimals = Token::from_addr(quote_mint)
        .get_decimals()
        .ok_or(AuthServerError::custom("quote token has no decimals"))?;

    let adjustment: BigDecimal = BigInt::from(10).pow(quote_decimals as u32).into();

    Ok(price / adjustment)
}

/// Overwrite the body of an HTTP response
pub fn overwrite_response_body<T: Serialize>(
    resp: &mut Response<Bytes>,
    body: T,
) -> Result<(), AuthServerError> {
    let body_bytes = Bytes::from(serde_json::to_vec(&body).map_err(AuthServerError::serde)?);

    resp.headers_mut().insert(CONTENT_LENGTH, body_bytes.len().into());
    *resp.body_mut() = body_bytes;

    Ok(())
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
