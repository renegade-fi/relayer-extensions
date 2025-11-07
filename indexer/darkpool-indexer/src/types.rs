//! Internal type definitions used throughout the darkpool indexer, used as the
//! canonical representations of data outside of the external API & DB layers.

// TODO: Find a better location for this module?

use alloy::primitives::Address;
use renegade_circuit_types::{Amount, balance::Balance, csprng::PoseidonCSPRNG, intent::Intent};
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::crypto_mocks::{
    recovery_stream::{create_recovery_seed_csprng, sample_nullifier},
    share_stream::create_share_seed_csprng,
};

// -------------
// | Constants |
// -------------

/// The name of the global (default) relayer matching pool
const GLOBAL_MATCHING_POOL: &str = "global";

// ---------
// | Types |
// ---------

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

    /// Generate the next expected state object for the account
    pub fn next_expected_state_object(&mut self) -> ExpectedStateObject {
        let recovery_stream_seed = self.recovery_seed_csprng.next().unwrap();
        let share_stream_seed = self.share_seed_csprng.next().unwrap();

        ExpectedStateObject::new(
            self.account_id,
            self.owner_address,
            recovery_stream_seed,
            share_stream_seed,
        )
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

        let expected_nullifier = sample_nullifier(&recovery_stream, 0 /* version */);

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
        share_stream_seed: Scalar,
        owner_address: Address,
        public_shares: Vec<Scalar>,
        private_shares: Vec<Scalar>,
    ) -> Self {
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        // New state objects are created at version 1
        recovery_stream.advance_by(1);

        let share_stream = PoseidonCSPRNG::new(share_stream_seed);

        let nullifier = sample_nullifier(&recovery_stream, 1 /* version */);

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

/// A balance state object
#[derive(Clone)]
pub struct BalanceStateObject {
    /// The underlying balance circuit type
    pub balance: Balance,
    /// The seed of the balance's recovery stream
    pub recovery_stream_seed: Scalar,
    /// The ID of the account which owns the balance
    pub account_id: Uuid,
    /// Whether the balance is active
    pub active: bool,
    /// Whether public fills are allowed against this balance
    pub allow_public_fills: bool,
}

impl BalanceStateObject {
    /// Create a new balance state object
    pub fn new(balance: Balance, recovery_stream_seed: Scalar, account_id: Uuid) -> Self {
        Self {
            balance,
            recovery_stream_seed,
            account_id,
            active: true,
            // We default to disallowing public fills on newly-created balances to err on the side
            // caution. This is a user decision that must be communicated from the
            // relayer.
            allow_public_fills: false,
        }
    }
}

/// An intent state object
#[derive(Clone)]
pub struct IntentStateObject {
    /// The underlying intent circuit type
    pub intent: Intent,
    /// The seed of the intent's recovery stream
    pub recovery_stream_seed: Scalar,
    /// The ID of the account which owns the intent
    pub account_id: Uuid,
    /// Whether the intent is active
    pub active: bool,
    /// The matching pool to which the intent is allocated
    pub matching_pool: String,
    /// Whether the intent allows external matches
    pub allow_external_matches: bool,
    /// The minimum fill size allowed for the intent
    pub min_fill_size: Amount,
    /// Whether to precompute a cancellation proof for the intent
    pub precompute_cancellation_proof: bool,
}

impl IntentStateObject {
    /// Create a new intent state object
    pub fn new(intent: Intent, recovery_stream_seed: Scalar, account_id: Uuid) -> Self {
        let min_fill_size = intent.amount_in;
        Self {
            intent,
            recovery_stream_seed,
            account_id,
            active: true,
            // We set the remaining metadata fields to reasonable, safe defaults.
            // These are user decisions that must be communicated from the relayer.
            matching_pool: GLOBAL_MATCHING_POOL.to_string(),
            allow_external_matches: false,
            min_fill_size,
            precompute_cancellation_proof: false,
        }
    }
}

// TODO: Define remaining internal types
