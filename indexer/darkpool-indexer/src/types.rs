//! Internal type definitions used throughout the darkpool indexer, used as the
//! canonical representations of data outside of the external API & DB layers.

// TODO: Find a better location for this module?

use alloy::primitives::Address;
use renegade_circuit_types::{
    Amount,
    balance::Balance,
    csprng::PoseidonCSPRNG,
    intent::Intent,
    state_wrapper::StateWrapper,
    traits::{BaseType, CircuitBaseType, SecretShareBaseType, SecretShareType},
};
use renegade_constants::Scalar;
use uuid::Uuid;

use crate::crypto_mocks::{
    recovery_stream::{create_recovery_seed_csprng, peek_nullifier, sample_next_nullifier},
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
#[derive(Clone)]
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
#[derive(Clone, PartialEq, Eq, Debug)]
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

        let expected_nullifier = peek_nullifier(&recovery_stream, 0 /* version */);

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
    /// The object's version is the previous index in the recovery stream.
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
    ) -> Self {
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        // New state objects are created after the first (0th) nullifier has been spent
        recovery_stream.advance_by(1);

        // Sample the object's current (unspent) nullifier, advancing the recovery
        // stream's state
        let nullifier = sample_next_nullifier(&mut recovery_stream);

        // Generate the private shares for the state object
        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed);
        let private_shares: Vec<Scalar> = share_stream.by_ref().take(public_shares.len()).collect();

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

    /// Update the generic state object with the given updated public shares
    pub fn update(&mut self, updated_public_shares: &[Scalar], updated_shares_index: usize) {
        // Sample the object's new (unspent) nullifier, advancing the recovery
        // stream's state
        let nullifier = sample_next_nullifier(&mut self.recovery_stream);
        self.nullifier = nullifier;

        // Overwrite the appropriate slice of the public & private shares
        let updated_private_shares: Vec<Scalar> =
            self.share_stream.by_ref().take(updated_public_shares.len()).collect();

        let start_index = updated_shares_index;
        let end_index = start_index + updated_public_shares.len();

        self.public_shares[start_index..end_index].copy_from_slice(updated_public_shares);
        self.private_shares[start_index..end_index].copy_from_slice(&updated_private_shares);
    }

    /// Reconstruct a circuit type from the public & private shares stored on
    /// the generic state object
    pub fn reconstruct_circuit_type<T>(&self) -> StateWrapper<T>
    where
        T: SecretShareBaseType + CircuitBaseType,
        T::ShareType: CircuitBaseType,
        <T::ShareType as SecretShareType>::Base: Into<T>,
    {
        let public_share = T::ShareType::from_scalars(&mut self.public_shares.clone().into_iter());
        let private_share =
            T::ShareType::from_scalars(&mut self.private_shares.clone().into_iter());

        let inner = public_share.add_shares(&private_share).into();

        StateWrapper {
            recovery_stream: self.recovery_stream.clone(),
            share_stream: self.share_stream.clone(),
            inner,
            public_share,
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

/// The data associated with a nullifier spend that is necessary for proper
/// indexing
#[derive(Clone)]
pub struct NullifierSpendData {
    /// The nullifier that was spent
    pub nullifier: Scalar,
    /// The block number in which the nullifier was spent
    pub block_number: u64,
    /// The type of the state object that was updated
    pub state_object_type: StateObjectType,
    /// The updated public shares of the state object
    pub updated_public_shares: Vec<Scalar>,
    /// The start index of the updated public shares within the secret-sharing
    /// of the state object
    pub updated_shares_index: usize,
}
