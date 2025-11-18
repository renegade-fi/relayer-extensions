//! Internal type definitions used throughout the darkpool indexer, used as the
//! canonical representations of data outside of the external API & DB layers.

// TODO: Find a better location for this module?

use alloy::primitives::{Address, B256};
use renegade_circuit_types::{
    Amount,
    balance::{Balance, BalanceShare},
    csprng::PoseidonCSPRNG,
    intent::Intent,
    state_wrapper::StateWrapper,
    traits::{BaseType, SecretShareType},
};
use renegade_constants::Scalar;
use renegade_crypto::fields::scalar_to_u128;
use uuid::Uuid;

use crate::crypto_mocks::{
    recovery_stream::create_recovery_seed_csprng, share_stream::create_share_seed_csprng,
    utils::decrypt_amount,
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

        ExpectedStateObject::new(self.account_id, recovery_stream_seed, share_stream_seed)
    }
}

/// A state object which is expected to be created
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedStateObject {
    /// The expected recovery ID
    pub recovery_id: Scalar,
    /// The ID of the account owning the state object associated with the
    /// nullifier
    pub account_id: Uuid,
    /// The recovery stream seed of the state object associated with the
    /// nullifier
    pub recovery_stream_seed: Scalar,
    /// The share stream seed of the state object associated with the
    /// nullifier
    pub share_stream_seed: Scalar,
}

impl ExpectedStateObject {
    /// Create a new expected state object
    pub fn new(account_id: Uuid, recovery_stream_seed: Scalar, share_stream_seed: Scalar) -> Self {
        let recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        let recovery_id = recovery_stream.get_ith(0);

        Self { recovery_id, account_id, recovery_stream_seed, share_stream_seed }
    }
}

/// A balance state object
#[derive(Clone)]
pub struct BalanceStateObject {
    /// The underlying balance circuit type
    pub balance: StateWrapper<Balance>,
    /// The ID of the account which owns the balance
    pub account_id: Uuid,
    /// Whether the balance is active
    pub active: bool,
}

impl BalanceStateObject {
    /// Create a new balance state object
    pub fn new(
        public_share: BalanceShare,
        recovery_stream_seed: Scalar,
        share_stream_seed: Scalar,
        account_id: Uuid,
    ) -> Self {
        // Compute the balance's private shares & reconstruct the plaintext
        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed);
        let private_share = BalanceShare::from_scalars(&mut share_stream);
        let balance_inner = public_share.add_shares(&private_share);

        // Ensure that the recovery stream has been advanced to indicate the usage of
        // the first recovery ID during the creation of the balance
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        recovery_stream.index = 1;

        let balance =
            StateWrapper { inner: balance_inner, recovery_stream, share_stream, public_share };

        Self { balance, account_id, active: true }
    }

    /// Update the balance amount using the given public share
    pub fn update_amount(&mut self, new_amount_public_share: Scalar) {
        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);

        // Update the public shares of the balance
        let mut public_share = self.balance.public_share();
        public_share.amount = new_amount_public_share;
        self.balance.public_share = public_share;

        // Update the plaintext balance amount
        let new_amount_private_share = self.balance.share_stream.next().unwrap();
        let new_amount_scalar = new_amount_public_share + new_amount_private_share;
        self.balance.inner.amount = scalar_to_u128(&new_amount_scalar);
    }

    /// Update the balance amount & fees using the given public shares
    pub fn update_amount_and_fees(
        &mut self,
        new_relayer_fee_public_share: Scalar,
        new_protocol_fee_public_share: Scalar,
        new_amount_public_share: Scalar,
    ) {
        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);

        // Update the public shares of the balance
        let mut public_share = self.balance.public_share();

        public_share.relayer_fee_balance = new_relayer_fee_public_share;
        public_share.protocol_fee_balance = new_protocol_fee_public_share;
        public_share.amount = new_amount_public_share;

        self.balance.public_share = public_share;

        // Update the plaintext balance fees & amount
        let share_stream = &mut self.balance.share_stream;
        let new_relayer_fee = decrypt_amount(new_relayer_fee_public_share, share_stream);
        let new_protocol_fee = decrypt_amount(new_protocol_fee_public_share, share_stream);
        let new_amount = decrypt_amount(new_amount_public_share, share_stream);

        self.balance.inner.relayer_fee_balance = new_relayer_fee;
        self.balance.inner.protocol_fee_balance = new_protocol_fee;
        self.balance.inner.amount = new_amount;
    }
}

/// An intent state object
#[derive(Clone)]
pub struct IntentStateObject {
    /// The underlying intent circuit type
    pub intent: StateWrapper<Intent>,
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

/// A public intent state object
#[derive(Clone)]
pub struct PublicIntentStateObject {
    /// The intent's hash
    pub intent_hash: B256,
    /// The underlying intent circuit type
    pub intent: Intent,
    /// The intent's version
    pub version: u64,
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

impl PublicIntentStateObject {
    /// Create a new public intent state object
    pub fn new(intent_hash: B256, intent: Intent, account_id: Uuid) -> Self {
        // Select safe default values for the public intent
        let version = 0;
        let active = true;
        let matching_pool = GLOBAL_MATCHING_POOL.to_string();
        let allow_external_matches = false;
        let min_fill_size = intent.amount_in;
        let precompute_cancellation_proof = false;

        Self {
            intent_hash,
            intent,
            version,
            account_id,
            active,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
        }
    }
}

// TODO: Define remaining internal types
