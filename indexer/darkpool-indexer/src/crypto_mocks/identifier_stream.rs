//! Helper functions for creating & sampling from identifier streams

use renegade_constants::Scalar;

use crate::crypto_mocks::{
    csprng::{PoseidonCSPRNG, compute_poseidon_hash},
    utils::hash_to_scalar,
};

// -------------
// | Constants |
// -------------

/// The message which is hashed alongside a master view seed to generate the
/// identifier seed CSPRNG seed
const IDENTIFIER_SEED_CSPRNG_MSG: &[u8] = b"identifier-seed-csprng";

// ---------------------
// | Utility Functions |
// ---------------------

/// Create the "identifier seed CSPRNG" from a given master view seed.
/// This is the CSPRNG from which identifier stream *seeds* are sampled for each
/// of the account's state objects.
fn create_identifier_seed_csprng(master_view_seed: Scalar) -> PoseidonCSPRNG {
    let mut seed_msg = master_view_seed.to_bytes_be();
    seed_msg.extend_from_slice(IDENTIFIER_SEED_CSPRNG_MSG);

    let csprng_seed = hash_to_scalar(&seed_msg);

    PoseidonCSPRNG::new(csprng_seed)
}

/// Sample the identifier stream seed for the n'th state object the account
/// associated with the given master view seed.
pub fn sample_identifier_seed(master_view_seed: Scalar, object_idx: usize) -> Scalar {
    let mut identifier_seed_csprng = create_identifier_seed_csprng(master_view_seed);
    identifier_seed_csprng.nth(object_idx).unwrap()
}

/// Sample the recovery ID for the given version of the state object with the
/// given identifier stream seed
pub fn sample_recovery_id(identifier_seed: Scalar, version: usize) -> Scalar {
    let mut recovery_id_csprng = PoseidonCSPRNG::new(identifier_seed);
    recovery_id_csprng.nth(version).unwrap()
}

/// Sample the nullifier for the given version of the state object with the
/// given identifier stream seed
pub fn sample_nullifier(identifier_seed: Scalar, version: usize) -> Scalar {
    let recovery_id = sample_recovery_id(identifier_seed, version);
    compute_poseidon_hash(&[recovery_id, identifier_seed])
}
