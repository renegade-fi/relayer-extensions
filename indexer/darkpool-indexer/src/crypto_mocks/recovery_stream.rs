//! Helper functions for working with recovery streams

use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_constants::Scalar;

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
