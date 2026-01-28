//! Helper functions for working with share streams

use renegade_constants::Scalar;
use renegade_darkpool_types::csprng::PoseidonCSPRNG;

use crate::crypto_mocks::utils::hash_to_scalar;

// -------------
// | Constants |
// -------------

/// The message which is hashed alongside a master view seed to generate the
/// share seed CSPRNG seed
const SHARE_SEED_CSPRNG_MSG: &[u8] = b"share-seed-csprng";

// ---------------------
// | Utility Functions |
// ---------------------

/// Create the "share seed CSPRNG" from a given master view seed.
/// This is the CSPRNG from which share stream *seeds* are sampled for each
/// of the account's state objects.
pub fn create_share_seed_csprng(master_view_seed: Scalar) -> PoseidonCSPRNG {
    let mut seed_msg = master_view_seed.to_bytes_be();
    seed_msg.extend_from_slice(SHARE_SEED_CSPRNG_MSG);

    let csprng_seed = hash_to_scalar(&seed_msg);

    PoseidonCSPRNG::new(csprng_seed)
}
