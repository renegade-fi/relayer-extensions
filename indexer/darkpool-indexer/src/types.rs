//! Internal type definitions used throughout the darkpool indexer, used as the
//! canonical representations of data outside of the external API & DB layers.

// TODO: Find a better location for this module?

use alloy_primitives::Address;
use darkpool_indexer_api::types::sqs::ApiStateObjectType;
use renegade_constants::Scalar;
use uuid::Uuid;

/// An account's master view seed
pub struct MasterViewSeed {
    /// The ID of the seed owner's account
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: Address,
    /// The master view seed
    pub seed: Scalar,
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
    /// The identifier stream seed of the state object associated with the
    /// nullifier
    pub identifier_seed: Scalar,
    /// The encryption cipher seed of the state object associated with the
    /// nullifier
    pub encryption_seed: Scalar,
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
    /// The object's identifier stream seed
    pub identifier_seed: Scalar,
    /// The ID of the account owning the state object
    pub account_id: Uuid,
    /// Whether the object is active
    pub active: bool,
    /// The type of the object
    pub object_type: StateObjectType,
    /// The object's current (unspent) nullifier
    pub nullifier: Scalar,
    /// The object's current version
    pub version: usize,
    /// The object's encryption cipher seed
    pub encryption_seed: Scalar,
    /// The current index of the object's encryption cipher
    pub encryption_cipher_index: usize,
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
        identifier_seed: Scalar,
        account_id: Uuid,
        object_type: StateObjectType,
        nullifier: Scalar,
        encryption_seed: Scalar,
        owner_address: Address,
        public_shares: Vec<Scalar>,
        private_shares: Vec<Scalar>,
    ) -> Self {
        Self {
            identifier_seed,
            account_id,
            active: true,
            object_type,
            nullifier,
            version: 1,
            encryption_seed,
            encryption_cipher_index: 0,
            owner_address,
            public_shares,
            private_shares,
        }
    }
}

// TODO: Define remaining internal types
