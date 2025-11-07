//! Internal type definitions used throughout the darkpool indexer, used as the
//! canonical representations of data outside of the external API & DB layers.

// TODO: Find a better location for this module?

use alloy_primitives::Address;
use darkpool_indexer_api::types::sqs::ApiStateObjectType;
use renegade_circuit_types::csprng::PoseidonCSPRNG;
use renegade_constants::Scalar;
use renegade_crypto::hash::compute_poseidon_hash;
use uuid::Uuid;

use crate::crypto_mocks::{
    recovery_stream::create_recovery_seed_csprng, share_stream::create_share_seed_csprng,
};
/// An account's master view seed
pub struct MasterViewSeed {
    /// The ID of the seed owner's account
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: Address,
    /// The master view seed
    pub seed: Scalar,
    /// The CSPRNG for recovery stream seeds
    pub recovery_seed_csprng: PoseidonCSPRNG,
    /// The CSPRNG for share stream seeds
    pub share_seed_csprng: PoseidonCSPRNG,
}

impl MasterViewSeed {
    /// Create a new master view seed
    pub fn new(account_id: Uuid, owner_address: Address, seed: Scalar) -> Self {
        let recovery_seed_csprng = create_recovery_seed_csprng(seed);
        let share_seed_csprng = create_share_seed_csprng(seed);

        Self { account_id, owner_address, seed, recovery_seed_csprng, share_seed_csprng }
    }
}

/// A state object which is expected to be created
pub struct ExpectedStateObject {
    /// The expected nullifier
    pub nullifier: Scalar,
    /// The ID of the account owning the state object associated with the
    /// nullifier
    pub account_id: Uuid,
    /// The address of the owner of the state object associated with the
    /// nullifier
    pub owner_address: Address,
    /// The recovery stream of the state object associated with the
    /// nullifier
    pub recovery_stream: PoseidonCSPRNG,
    /// The share stream of the state object associated with the
    /// nullifier
    pub share_stream: PoseidonCSPRNG,
}

impl ExpectedStateObject {
    /// Create a new expected state object
    pub fn new(
        account_id: Uuid,
        owner_address: Address,
        recovery_stream_seed: Scalar,
        share_stream_seed: Scalar,
    ) -> Self {
        let recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        let share_stream = PoseidonCSPRNG::new(share_stream_seed);

        let expected_recovery_id = recovery_stream.get_ith(0);
        let expected_nullifier =
            compute_poseidon_hash(&[expected_recovery_id, recovery_stream_seed]);

        Self {
            nullifier: expected_nullifier,
            account_id,
            owner_address,
            recovery_stream,
            share_stream,
        }
    }
}

/// The type of a state object
#[derive(Clone)]
pub enum StateObjectType {
    /// An intent state object
    Intent,
    /// A balance state object
    Balance,
}

impl From<ApiStateObjectType> for StateObjectType {
    fn from(value: ApiStateObjectType) -> Self {
        match value {
            ApiStateObjectType::Intent => Self::Intent,
            ApiStateObjectType::Balance => Self::Balance,
        }
    }
}

/// A generic state object, containing just the raw public/private shares & no
/// object-specific metadata
#[derive(Clone)]
pub struct GenericStateObject {
    /// The object's recovery stream.
    ///
    /// The stream's index is, equivalently, the object's version.
    pub recovery_stream: PoseidonCSPRNG,
    /// The object's share stream
    pub share_stream: PoseidonCSPRNG,
    /// The ID of the account owning the state object
    pub account_id: Uuid,
    /// Whether the object is active
    pub active: bool,
    /// The type of the object
    pub object_type: StateObjectType,
    /// The object's current (unspent) nullifier
    pub nullifier: Scalar,
    /// The address of the object's owner
    pub owner_address: Address,
    /// The public shares of the object
    pub public_shares: Vec<Scalar>,
    /// The private shares of the object
    pub private_shares: Vec<Scalar>,
}

impl GenericStateObject {
    /// Create a new generic state object
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        recovery_stream_seed: Scalar,
        account_id: Uuid,
        object_type: StateObjectType,
        nullifier: Scalar,
        share_stream_seed: Scalar,
        owner_address: Address,
        public_shares: Vec<Scalar>,
        private_shares: Vec<Scalar>,
    ) -> Self {
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        // New state objects are created at version 1
        recovery_stream.advance_by(1);

        let share_stream = PoseidonCSPRNG::new(share_stream_seed);

        Self {
            recovery_stream,
            account_id,
            active: true,
            object_type,
            nullifier,
            share_stream,
            owner_address,
            public_shares,
            private_shares,
        }
    }
}

// TODO: Define remaining internal types
