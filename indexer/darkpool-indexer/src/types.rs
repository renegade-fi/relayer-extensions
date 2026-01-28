//! Internal type definitions used throughout the darkpool indexer, used as the
//! canonical representations of data outside of the external API & DB layers.

use alloy::primitives::{Address, B256};
use darkpool_indexer_api::types::http::{ApiBalance, ApiIntent, ApiPublicIntent, ApiStateObject};
use renegade_circuit_types::{
    Amount,
    traits::{BaseType, SecretShareType},
};
use renegade_constants::Scalar;
use renegade_darkpool_types::{
    balance::{DarkpoolBalanceShare, DarkpoolStateBalance, PostMatchBalanceShare},
    csprng::PoseidonCSPRNG,
    fee::FeeTake,
    intent::{DarkpoolStateIntent, Intent, IntentShare},
    settlement_obligation::SettlementObligation,
};
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
    pub balance: DarkpoolStateBalance,
    /// The ID of the account which owns the balance
    pub account_id: Uuid,
    /// Whether the balance is active
    pub active: bool,
}

impl BalanceStateObject {
    /// Create a new balance state object
    pub fn new(
        public_share: DarkpoolBalanceShare,
        recovery_stream_seed: Scalar,
        share_stream_seed: Scalar,
        account_id: Uuid,
    ) -> Self {
        // Compute the balance's private shares & reconstruct the plaintext
        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed);
        let private_share = DarkpoolBalanceShare::from_scalars(&mut share_stream);
        let balance_inner = public_share.add_shares(&private_share);

        // Ensure that the recovery stream has been advanced to indicate the usage of
        // the first recovery ID during the creation of the balance
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        recovery_stream.index = 1;

        let balance = DarkpoolStateBalance {
            inner: balance_inner,
            recovery_stream,
            share_stream,
            public_share,
        };

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
        let share_stream = &mut self.balance.share_stream;
        let new_amount = decrypt_amount(new_amount_public_share, share_stream);
        self.balance.inner.amount = new_amount;
    }

    /// Update the balance as the input balance in the first fill of a
    /// public-fill match settlement
    ///
    /// TODO: The authority field type has changed from Address to
    /// SchnorrPublicKey. The new_one_time_authority_share parameter and
    /// authority update logic needs to be redesigned for the new type
    /// system.
    pub fn update_from_public_first_fill_as_input_balance(
        &mut self,
        settlement_obligation: &SettlementObligation,
        _new_one_time_authority_share: Scalar,
    ) {
        // TODO: Authority update logic needs redesign for SchnorrPublicKey type.
        // Previously this would decrypt an Address from the share and update both
        // the inner authority and public_share.authority fields.

        // Re-encrypt the updated balance shares
        self.balance.reencrypt_post_match_share();

        // Apply the settlement obligation to the balance
        self.balance.apply_obligation_in_balance(settlement_obligation);

        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);
    }

    /// Update the balance as the input balance of a public-fill match
    /// settlement
    pub fn update_from_public_fill_as_input_balance(
        &mut self,
        settlement_obligation: &SettlementObligation,
    ) {
        // Re-encrypt the updated balance shares
        self.balance.reencrypt_post_match_share();

        // Apply the settlement obligation to the balance
        self.balance.apply_obligation_in_balance(settlement_obligation);

        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);
    }

    /// Update the balance as the output balance of a public-fill match
    /// settlement
    pub fn update_from_public_fill_as_output_balance(
        &mut self,
        settlement_obligation: &SettlementObligation,
        fee_take: &FeeTake,
    ) {
        // Re-encrypt the updated balance shares
        self.balance.reencrypt_post_match_share();

        // Apply the settlement obligation to the balance
        self.balance.apply_obligation_out_balance(settlement_obligation, fee_take);

        // Note, we don't need to accrue fees into the balance, since fees are
        // transferred immediately in public-fill settlement.

        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);
    }

    /// Apply the balance updates resulting from a private-fill match settlement
    pub fn update_from_private_fill(&mut self, post_match_balance_share: &PostMatchBalanceShare) {
        let PostMatchBalanceShare { relayer_fee_balance, protocol_fee_balance, amount } =
            post_match_balance_share;

        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);

        // Update the public shares of the balance
        let mut public_share = self.balance.public_share();

        public_share.relayer_fee_balance = *relayer_fee_balance;
        public_share.protocol_fee_balance = *protocol_fee_balance;
        public_share.amount = *amount;

        self.balance.public_share = public_share;

        // Update the plaintext balance fees & amount
        let share_stream = &mut self.balance.share_stream;
        let new_relayer_fee = decrypt_amount(*relayer_fee_balance, share_stream);
        let new_protocol_fee = decrypt_amount(*protocol_fee_balance, share_stream);
        let new_amount = decrypt_amount(*amount, share_stream);

        self.balance.inner.relayer_fee_balance = new_relayer_fee;
        self.balance.inner.protocol_fee_balance = new_protocol_fee;
        self.balance.inner.amount = new_amount;
    }

    /// Update the protocol fee amount using the given public share
    pub fn update_protocol_fee(&mut self, new_protocol_fee_public_share: Scalar) {
        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);

        // Update the public shares of the balance
        let mut public_share = self.balance.public_share();
        public_share.protocol_fee_balance = new_protocol_fee_public_share;
        self.balance.public_share = public_share;

        // Update the plaintext protocol fee
        let share_stream = &mut self.balance.share_stream;
        let new_protocol_fee = decrypt_amount(new_protocol_fee_public_share, share_stream);

        self.balance.inner.protocol_fee_balance = new_protocol_fee;
    }

    /// Update the relayer fee amount using the given public share
    pub fn update_relayer_fee(&mut self, new_relayer_fee_public_share: Scalar) {
        // Advance the recovery stream to indicate the next object version
        self.balance.recovery_stream.advance_by(1);

        // Update the public shares of the balance
        let mut public_share = self.balance.public_share();
        public_share.relayer_fee_balance = new_relayer_fee_public_share;
        self.balance.public_share = public_share;

        // Update the plaintext relayer fee
        let share_stream = &mut self.balance.share_stream;
        let new_relayer_fee = decrypt_amount(new_relayer_fee_public_share, share_stream);

        self.balance.inner.relayer_fee_balance = new_relayer_fee;
    }
}

impl From<BalanceStateObject> for ApiBalance {
    fn from(value: BalanceStateObject) -> Self {
        let BalanceStateObject { balance, .. } = value;
        ApiBalance { balance }
    }
}

impl From<BalanceStateObject> for ApiStateObject {
    fn from(value: BalanceStateObject) -> Self {
        let api_balance: ApiBalance = value.into();
        api_balance.into()
    }
}

/// A struct representing the input/output amounts parsed from a public
/// obligation bundle associated with a match settlement
#[derive(Clone)]
pub struct ObligationAmounts {
    /// The input amount on the obligation bundle
    pub amount_in: Scalar,
    /// The output amount on the obligation bundle
    pub amount_out: Scalar,
}

/// An intent state object
#[derive(Clone)]
pub struct IntentStateObject {
    /// The underlying intent circuit type
    pub intent: DarkpoolStateIntent,
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
    pub fn new(
        public_share: IntentShare,
        recovery_stream_seed: Scalar,
        share_stream_seed: Scalar,
        account_id: Uuid,
    ) -> Self {
        // Compute the intent's private shares & reconstruct the plaintext
        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed);
        let private_share = IntentShare::from_scalars(&mut share_stream);
        let intent_inner = public_share.add_shares(&private_share);

        // Ensure that the recovery stream has been advanced to indicate the usage of
        // the first recovery ID during the creation of the intent
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed);
        recovery_stream.index = 1;

        let intent = DarkpoolStateIntent {
            inner: intent_inner,
            recovery_stream,
            share_stream,
            public_share,
        };

        // Select safe default values for the intent metadata
        let matching_pool = GLOBAL_MATCHING_POOL.to_string();
        let allow_external_matches = false;
        let min_fill_size = intent.inner.amount_in;
        let precompute_cancellation_proof = false;

        Self {
            intent,
            account_id,
            active: true,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
        }
    }

    /// Update the intent amount using the given public share
    pub fn update_amount(&mut self, new_amount_public_share: Scalar) {
        // Advance the recovery stream to indicate the next object version
        self.intent.recovery_stream.advance_by(1);

        // Update the public shares of the intent
        let mut public_share = self.intent.public_share();
        public_share.amount_in = new_amount_public_share;
        self.intent.public_share = public_share;

        // Update the plaintext intent amount
        let share_stream = &mut self.intent.share_stream;
        let new_amount = decrypt_amount(new_amount_public_share, share_stream);
        self.intent.inner.amount_in = new_amount;
    }

    /// Update the intent from a settlement obligation
    pub fn update_from_settlement_obligation(
        &mut self,
        settlement_obligation: &SettlementObligation,
    ) {
        // Re-encrypt the updated intent shares
        self.intent.reencrypt_amount_in();

        // Apply the settlement obligation to the intent
        self.intent.apply_settlement_obligation(settlement_obligation);

        // Advance the recovery stream to indicate the next object version
        self.intent.recovery_stream.advance_by(1);
    }

    /// Cancel the intent
    pub fn cancel(&mut self) {
        self.active = false;
    }
}

impl From<IntentStateObject> for ApiIntent {
    fn from(value: IntentStateObject) -> Self {
        let IntentStateObject {
            intent,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
            ..
        } = value;

        ApiIntent {
            intent,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
        }
    }
}

impl From<IntentStateObject> for ApiStateObject {
    fn from(value: IntentStateObject) -> Self {
        let api_intent: ApiIntent = value.into();
        api_intent.into()
    }
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
        // Select safe default values for the public intent metadata
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

impl From<PublicIntentStateObject> for ApiPublicIntent {
    fn from(value: PublicIntentStateObject) -> Self {
        let PublicIntentStateObject {
            intent_hash,
            intent,
            version,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
            ..
        } = value;

        ApiPublicIntent {
            intent_hash,
            intent,
            version,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
        }
    }
}

impl From<PublicIntentStateObject> for ApiStateObject {
    fn from(value: PublicIntentStateObject) -> Self {
        let api_public_intent: ApiPublicIntent = value.into();
        api_public_intent.into()
    }
}
