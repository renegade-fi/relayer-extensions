//! Helper functions for creating & sampling from encryption stream ciphers

use renegade_constants::Scalar;

use crate::crypto_mocks::{csprng::PoseidonCSPRNG, utils::hash_to_scalar};

// -------------
// | Constants |
// -------------

/// The message which is hashed alongside a master view seed to generate the
/// encryption seed CSPRNG seed
const ENCRYPTION_SEED_CSPRNG_MSG: &[u8] = b"encryption-seed-csprng";

// ---------------------
// | Utility Functions |
// ---------------------

/// Create the "encryption seed CSPRNG" from a given master view seed.
/// This is the CSPRNG from which encryption stream *seeds* are sampled for each
/// of the account's state objects.
fn create_encryption_seed_csprng(master_view_seed: Scalar) -> PoseidonCSPRNG {
    let mut seed_msg = master_view_seed.to_bytes_be();
    seed_msg.extend_from_slice(ENCRYPTION_SEED_CSPRNG_MSG);

    let csprng_seed = hash_to_scalar(&seed_msg);

    PoseidonCSPRNG::new(csprng_seed)
}

/// Sample the encryption stream seed for the n'th state object the account
/// associated with the given master view seed.
pub fn sample_encryption_seed(master_view_seed: Scalar, object_idx: usize) -> Scalar {
    let mut encryption_seed_csprng = create_encryption_seed_csprng(master_view_seed);
    encryption_seed_csprng.nth(object_idx).unwrap()
}
