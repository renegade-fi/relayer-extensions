//! Common, low-level cryptographic utilities

use alloy::primitives::{B256, keccak256};
use renegade_circuit_types::{Amount, csprng::PoseidonCSPRNG};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;

/// Hash a message to a scalar. We do this by hashing the message, extending the
/// hash to 64 bytes, then performing modular reduction of the result into a
/// scalar.
///
/// We do this to ensure a uniform sampling of the scalar field.
pub fn hash_to_scalar(msg: &[u8]) -> Scalar {
    // Hash the message
    let msg_hash = keccak256(msg);

    // Hash the hash again
    let recursive_hash = keccak256(&msg_hash);

    // Concatenate the hashes
    let mut extended_hash = [0u8; B256::len_bytes() * 2];
    extended_hash[..B256::len_bytes()].copy_from_slice(msg_hash.as_slice());
    extended_hash[B256::len_bytes()..].copy_from_slice(recursive_hash.as_slice());

    // Perform modular reduction
    Scalar::from_be_bytes_mod_order(&extended_hash)
}

/// Decrypt an amount ciphertext using a stream cipher, advancing its internal
/// state
pub fn decrypt_amount(amount_public_share: Scalar, stream_cipher: &mut PoseidonCSPRNG) -> Amount {
    let private_share = stream_cipher.next().unwrap();
    let amount_scalar = amount_public_share + private_share;
    scalar_to_u128(&amount_scalar)
}
