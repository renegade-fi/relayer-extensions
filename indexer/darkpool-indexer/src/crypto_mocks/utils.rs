//! Common, low-level cryptographic utilities

use renegade_circuit_types::{Amount, csprng::PoseidonCSPRNG};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;
use tiny_keccak::Hasher;

/// The output size of the Keccak-256 hash function in bytes
const KECCAK_OUTPUT_SIZE: usize = 32;

/// Compute the Keccak-256 hash of a message
pub fn keccak256(msg: &[u8]) -> [u8; KECCAK_OUTPUT_SIZE] {
    let mut hash = [0u8; KECCAK_OUTPUT_SIZE];

    let mut hasher = tiny_keccak::Keccak::v256();
    hasher.update(msg);
    hasher.finalize(&mut hash);

    hash
}

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
    let mut extended_hash = [0u8; 64];
    extended_hash[..KECCAK_OUTPUT_SIZE].copy_from_slice(&msg_hash);
    extended_hash[KECCAK_OUTPUT_SIZE..].copy_from_slice(&recursive_hash);

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
