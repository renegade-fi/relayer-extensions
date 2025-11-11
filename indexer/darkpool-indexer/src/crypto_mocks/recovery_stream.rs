//! Helper functions for working with recovery streams

use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_constants::Scalar;
use renegade_crypto::hash::compute_poseidon_hash;

use crate::crypto_mocks::utils::hash_to_scalar;

// -------------
// | Constants |
// -------------

/// The message which is hashed alongside a master view seed to generate the
/// recovery seed CSPRNG seed
const RECOVERY_SEED_CSPRNG_MSG: &[u8] = b"recovery-seed-csprng";

// ---------------------
// | Utility Functions |
// ---------------------

/// Create the "recovery seed CSPRNG" from a given master view seed.
/// This is the CSPRNG from which recovery stream *seeds* are sampled for each
/// of the account's state objects.
pub fn create_recovery_seed_csprng(master_view_seed: Scalar) -> PoseidonCSPRNG {
    let mut seed_msg = master_view_seed.to_bytes_be();
    seed_msg.extend_from_slice(RECOVERY_SEED_CSPRNG_MSG);

    let csprng_seed = hash_to_scalar(&seed_msg);

    PoseidonCSPRNG::new(csprng_seed)
}

/// Generate the nullifier for the given object version from the provided
/// recovery stream, without mutating its state
pub fn peek_nullifier(recovery_stream: &PoseidonCSPRNG, version: u64) -> Scalar {
    let recovery_id = recovery_stream.get_ith(version);
    compute_poseidon_hash(&[recovery_id, recovery_stream.seed])
}

/// Sample the next nullifier from the provided recovery stream, advancing its
/// state
pub fn sample_next_nullifier(recovery_stream: &mut PoseidonCSPRNG) -> Scalar {
    let recovery_id = recovery_stream.next().unwrap();
    compute_poseidon_hash(&[recovery_id, recovery_stream.seed])
}
