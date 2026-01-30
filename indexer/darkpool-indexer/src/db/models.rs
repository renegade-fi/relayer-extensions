//! Type bindings for the indexer's database table records

use std::str::FromStr;

use alloy::primitives::{Address, B256};
use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{
    Selectable,
    prelude::{AsChangeset, Insertable, Queryable},
};
use renegade_circuit_types::{primitives::schnorr::SchnorrPublicKey, traits::BaseType};
use renegade_constants::Scalar;
use renegade_darkpool_types::{
    balance::{DarkpoolBalance, DarkpoolBalanceShare, DarkpoolStateBalance},
    csprng::PoseidonCSPRNG,
    intent::{DarkpoolStateIntent, Intent, IntentShare},
    state_wrapper::StateWrapper,
};
use renegade_types_account::account::order::{Order, OrderMetadata, PrivacyRing};
use uuid::Uuid;

use crate::{
    crypto_mocks::{
        recovery_stream::create_recovery_seed_csprng, share_stream::create_share_seed_csprng,
    },
    db::utils::{
        bigdecimal_to_fixed_point, bigdecimal_to_scalar, fixed_point_to_bigdecimal,
        scalar_to_bigdecimal,
    },
    types::{
        BalanceStateObject, ExpectedStateObject, IntentStateObject, MasterViewSeed,
        PublicIntentStateObject,
    },
};

// ----------------
// | Table Models |
// ----------------

// === Master View Seeds Table ===

/// A master view seed record
#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::db::schema::master_view_seeds)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MasterViewSeedModel {
    /// The ID of the seed owner's account
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: String,
    /// The master view seed
    pub seed: BigDecimal,
    /// The index of the recovery seed CSPRNG
    pub recovery_seed_csprng_index: i64,
    /// The index of the share seed CSPRNG
    pub share_seed_csprng_index: i64,
}

impl From<MasterViewSeed> for MasterViewSeedModel {
    fn from(value: MasterViewSeed) -> Self {
        let MasterViewSeed {
            account_id,
            owner_address,
            seed,
            recovery_seed_csprng,
            share_seed_csprng,
        } = value;

        let seed_bigdecimal = scalar_to_bigdecimal(seed);
        let owner_address_string = owner_address.to_string();

        let recovery_seed_csprng_index_bigdecimal = recovery_seed_csprng.index as i64;
        let share_seed_csprng_index_bigdecimal = share_seed_csprng.index as i64;

        MasterViewSeedModel {
            account_id,
            owner_address: owner_address_string,
            seed: seed_bigdecimal,
            recovery_seed_csprng_index: recovery_seed_csprng_index_bigdecimal,
            share_seed_csprng_index: share_seed_csprng_index_bigdecimal,
        }
    }
}

impl From<MasterViewSeedModel> for MasterViewSeed {
    fn from(value: MasterViewSeedModel) -> Self {
        let MasterViewSeedModel {
            account_id,
            owner_address,
            seed,
            recovery_seed_csprng_index,
            share_seed_csprng_index,
        } = value;

        let seed_scalar = bigdecimal_to_scalar(seed);
        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let recovery_seed_csprng_index_u64 = recovery_seed_csprng_index as u64;
        let share_seed_csprng_index_u64 = share_seed_csprng_index as u64;

        let mut recovery_seed_csprng = create_recovery_seed_csprng(seed_scalar);
        let mut share_seed_csprng = create_share_seed_csprng(seed_scalar);

        recovery_seed_csprng.index = recovery_seed_csprng_index_u64;
        share_seed_csprng.index = share_seed_csprng_index_u64;

        MasterViewSeed {
            account_id,
            owner_address: owner_address_address,
            seed: seed_scalar,
            recovery_seed_csprng,
            share_seed_csprng,
        }
    }
}

// === Expected State Objects Table ===

/// An expected state object record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::expected_state_objects)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ExpectedStateObjectModel {
    /// The expected recovery ID
    pub recovery_id: BigDecimal,
    /// The ID of the account owning the state object associated with the
    /// nullifier
    pub account_id: Uuid,
    /// The recovery stream seed of the state object associated with the
    /// nullifier
    pub recovery_stream_seed: BigDecimal,
    /// The share stream seed of the state object associated with the
    /// nullifier
    pub share_stream_seed: BigDecimal,
}

impl From<ExpectedStateObject> for ExpectedStateObjectModel {
    fn from(value: ExpectedStateObject) -> Self {
        let ExpectedStateObject {
            recovery_id,
            account_id,
            recovery_stream_seed,
            share_stream_seed,
        } = value;

        let recovery_id_bigdecimal = scalar_to_bigdecimal(recovery_id);
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);
        let share_stream_seed_bigdecimal = scalar_to_bigdecimal(share_stream_seed);

        ExpectedStateObjectModel {
            recovery_id: recovery_id_bigdecimal,
            account_id,
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            share_stream_seed: share_stream_seed_bigdecimal,
        }
    }
}

impl From<ExpectedStateObjectModel> for ExpectedStateObject {
    fn from(value: ExpectedStateObjectModel) -> Self {
        let ExpectedStateObjectModel {
            recovery_id,
            account_id,
            recovery_stream_seed,
            share_stream_seed,
        } = value;

        let recovery_id_scalar = bigdecimal_to_scalar(recovery_id);
        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);
        let share_stream_seed_scalar = bigdecimal_to_scalar(share_stream_seed);

        ExpectedStateObject {
            recovery_id: recovery_id_scalar,
            account_id,
            recovery_stream_seed: recovery_stream_seed_scalar,
            share_stream_seed: share_stream_seed_scalar,
        }
    }
}

// === Processed Nullifiers Table ===

/// A processed nullifier record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::processed_nullifiers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedNullifierModel {
    /// The nullifier
    pub nullifier: BigDecimal,
    /// The block number in which the nullifier was spent
    pub block_number: i64,
}

// === Processed Recovery IDs Table ===

/// A processed recovery ID record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::processed_recovery_ids)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedRecoveryIDModel {
    /// The recovery ID
    pub recovery_id: BigDecimal,
    /// The block number in which the recovery ID was processed
    pub block_number: i64,
}

// === Processed Public Intent Creations Table ===

/// A processed public intent creation record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::processed_public_intent_creations)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedPublicIntentCreationModel {
    /// The public intent's hash
    pub intent_hash: String,
    /// The transaction hash in which the public intent was created
    pub tx_hash: String,
    /// The block number in which the public intent was created
    pub block_number: i64,
}

// === Processed Public Intent Updates Table ===

/// A processed public intent update record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::processed_public_intent_updates)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedPublicIntentUpdateModel {
    /// The public intent's hash
    pub intent_hash: String,
    /// The transaction hash in which the public intent was updated
    pub tx_hash: String,
    /// The block number in which the public intent was updated
    pub block_number: i64,
}

// === Intents Table ===

/// An intent record
#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::db::schema::intents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct IntentModel {
    /// The intent's recovery stream seed
    pub recovery_stream_seed: BigDecimal,
    /// The intent's version
    pub version: i64,
    /// The intent's share stream seed
    pub share_stream_seed: BigDecimal,
    /// The intent's share stream index
    pub share_stream_index: i64,
    /// The intent's current (unspent) nullifier
    pub nullifier: BigDecimal,
    /// The intent's public shares
    pub public_shares: Vec<BigDecimal>,
    /// The mint of the input token in the intent
    pub input_mint: String,
    /// The mint of the output token in the intent
    pub output_mint: String,
    /// The address of the intent's owner
    pub owner_address: String,
    /// The minimum price at which the intent can be filled
    pub min_price: BigDecimal,
    /// The amount of the input token to be traded via the intent
    pub input_amount: BigDecimal,
    /// The ID of the account owning the intent
    pub account_id: Uuid,
    /// Whether the intent is active
    pub active: bool,
    /// The matching pool to which the intent is allocated
    pub matching_pool: String,
    /// Whether the intent allows external matches
    pub allow_external_matches: bool,
    /// The minimum fill size allowed for the intent
    pub min_fill_size: BigDecimal,
    /// Whether to precompute a cancellation proof for the intent
    pub precompute_cancellation_proof: bool,
}

impl From<IntentStateObject> for IntentModel {
    fn from(value: IntentStateObject) -> Self {
        let nullifier_bigdecimal = scalar_to_bigdecimal(value.intent.compute_nullifier());

        let IntentStateObject {
            intent:
                DarkpoolStateIntent {
                    inner: Intent { in_token, out_token, owner, min_price, amount_in },
                    recovery_stream,
                    share_stream,
                    public_share,
                },
            account_id,
            active,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
        } = value;

        let input_mint_string = in_token.to_string();
        let output_mint_string = out_token.to_string();
        let owner_address_string = owner.to_string();
        let min_price_bigdecimal = fixed_point_to_bigdecimal(min_price);
        let input_amount_bigdecimal = amount_in.into();

        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream.seed);
        // The intent's version is the previous index in the recovery stream
        let version_i64 = (recovery_stream.index - 1) as i64;

        let share_stream_seed_bigdecimal = scalar_to_bigdecimal(share_stream.seed);
        let share_stream_index_i64 = share_stream.index as i64;

        let public_shares_bigdecimals =
            public_share.to_scalars().into_iter().map(scalar_to_bigdecimal).collect();

        let min_fill_size_bigdecimal = min_fill_size.into();

        IntentModel {
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            version: version_i64,
            share_stream_seed: share_stream_seed_bigdecimal,
            share_stream_index: share_stream_index_i64,
            nullifier: nullifier_bigdecimal,
            public_shares: public_shares_bigdecimals,
            account_id,
            active,
            input_mint: input_mint_string,
            output_mint: output_mint_string,
            owner_address: owner_address_string,
            min_price: min_price_bigdecimal,
            input_amount: input_amount_bigdecimal,
            matching_pool,
            allow_external_matches,
            min_fill_size: min_fill_size_bigdecimal,
            precompute_cancellation_proof,
        }
    }
}

impl From<IntentModel> for IntentStateObject {
    fn from(value: IntentModel) -> Self {
        let IntentModel {
            recovery_stream_seed,
            version,
            share_stream_seed,
            share_stream_index,
            // We don't need the nullifier, it can be computed from the circuit type
            nullifier: _nullifier,
            public_shares,
            account_id,
            active,
            input_mint,
            output_mint,
            owner_address,
            min_price,
            input_amount,
            matching_pool,
            allow_external_matches,
            min_fill_size,
            precompute_cancellation_proof,
        } = value;

        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);
        let version_u64 = version as u64;
        // The intent's recovery stream index is always one more than the version
        let recovery_stream_index = version_u64 + 1;
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed_scalar);
        recovery_stream.index = recovery_stream_index;

        let share_stream_seed_scalar = bigdecimal_to_scalar(share_stream_seed);
        let share_stream_index_u64 = share_stream_index as u64;

        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed_scalar);
        share_stream.index = share_stream_index_u64;

        let public_shares_scalars =
            IntentShare::from_scalars(&mut public_shares.into_iter().map(bigdecimal_to_scalar));

        let input_mint_address =
            Address::from_str(&input_mint).expect("Input mint must be a valid address");

        let output_mint_address =
            Address::from_str(&output_mint).expect("Output mint must be a valid address");

        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let min_price_fixed_point = bigdecimal_to_fixed_point(min_price);
        let input_amount_u128 =
            input_amount.to_u128().expect("Input amount cannot be converted to u128");

        let min_fill_size_u128 =
            min_fill_size.to_u128().expect("Min fill size cannot be converted to u128");

        IntentStateObject {
            intent: DarkpoolStateIntent {
                inner: Intent {
                    in_token: input_mint_address,
                    out_token: output_mint_address,
                    owner: owner_address_address,
                    min_price: min_price_fixed_point,
                    amount_in: input_amount_u128,
                },
                recovery_stream,
                share_stream,
                public_share: public_shares_scalars,
            },
            account_id,
            active,
            matching_pool,
            allow_external_matches,
            min_fill_size: min_fill_size_u128,
            precompute_cancellation_proof,
        }
    }
}

// === Public Intents Table ===

/// A public intent record
#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::db::schema::public_intents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PublicIntentModel {
    /// The intent's hash
    pub intent_hash: String,
    /// The order's ID
    pub order_id: Uuid,
    /// The mint of the input token in the intent
    pub input_mint: String,
    /// The mint of the output token in the intent
    pub output_mint: String,
    /// The address of the intent's owner
    pub owner_address: String,
    /// The minimum price at which the intent can be filled
    pub min_price: BigDecimal,
    /// The amount of the input token to be traded via the intent
    pub input_amount: BigDecimal,
    /// The ID of the account owning the intent
    pub account_id: Uuid,
    /// Whether the intent is active
    pub active: bool,
    /// The matching pool to which the intent is allocated
    pub matching_pool: String,
    /// Whether the intent allows external matches
    pub allow_external_matches: bool,
    /// The minimum fill size allowed for the intent
    pub min_fill_size: BigDecimal,
}

impl From<PublicIntentStateObject> for PublicIntentModel {
    fn from(value: PublicIntentStateObject) -> Self {
        let PublicIntentStateObject { intent_hash, order, account_id, matching_pool, active } =
            value;

        // Extract intent fields from the order
        let Intent { in_token, out_token, owner, min_price, amount_in } = order.intent.inner;

        // Extract metadata fields from the order
        let OrderMetadata { min_fill_size, allow_external_matches } = order.metadata;

        let intent_hash_string = intent_hash.to_string();
        let input_mint_string = in_token.to_string();
        let output_mint_string = out_token.to_string();
        let owner_address_string = owner.to_string();
        let min_price_bigdecimal = fixed_point_to_bigdecimal(min_price);
        let input_amount_bigdecimal = amount_in.into();
        let min_fill_size_bigdecimal = min_fill_size.into();

        PublicIntentModel {
            intent_hash: intent_hash_string,
            order_id: order.id,
            account_id,
            active,
            input_mint: input_mint_string,
            output_mint: output_mint_string,
            owner_address: owner_address_string,
            min_price: min_price_bigdecimal,
            input_amount: input_amount_bigdecimal,
            matching_pool,
            allow_external_matches,
            min_fill_size: min_fill_size_bigdecimal,
        }
    }
}

impl From<PublicIntentModel> for PublicIntentStateObject {
    fn from(value: PublicIntentModel) -> Self {
        let PublicIntentModel {
            intent_hash,
            order_id,
            account_id,
            active,
            input_mint,
            output_mint,
            owner_address,
            min_price,
            input_amount,
            matching_pool,
            allow_external_matches,
            min_fill_size,
        } = value;

        let intent_hash_b256 =
            B256::from_str(&intent_hash).expect("Intent hash must be a valid B256");

        let input_mint_address =
            Address::from_str(&input_mint).expect("Input mint must be a valid address");

        let output_mint_address =
            Address::from_str(&output_mint).expect("Output mint must be a valid address");

        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let min_price_fixed_point = bigdecimal_to_fixed_point(min_price);
        let input_amount_u128 =
            input_amount.to_u128().expect("Input amount cannot be converted to u128");

        let min_fill_size_u128 =
            min_fill_size.to_u128().expect("Min fill size cannot be converted to u128");

        // Reconstruct the intent
        let intent = Intent {
            in_token: input_mint_address,
            out_token: output_mint_address,
            owner: owner_address_address,
            min_price: min_price_fixed_point,
            amount_in: input_amount_u128,
        };

        // Reconstruct the state wrapper with zero seeds (Ring0 intents don't use
        // secret shares or recovery streams)
        let state_intent = StateWrapper::new(intent, Scalar::zero(), Scalar::zero());

        // Reconstruct the order metadata
        let metadata = OrderMetadata::new(min_fill_size_u128, allow_external_matches);

        // Reconstruct the order with Ring0 privacy level
        let order = Order::new_with_ring(order_id, state_intent, metadata, PrivacyRing::Ring0);

        PublicIntentStateObject {
            intent_hash: intent_hash_b256,
            order,
            account_id,
            matching_pool,
            active,
        }
    }
}

// === Balances Table ===

/// A balance record
#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::db::schema::balances)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BalanceModel {
    /// The balance's recovery stream seed
    pub recovery_stream_seed: BigDecimal,
    /// The balance's version
    pub version: i64,
    /// The balance's share stream seed
    pub share_stream_seed: BigDecimal,
    /// The balance's share stream index
    pub share_stream_index: i64,
    /// The balance's current (unspent) nullifier
    pub nullifier: BigDecimal,
    /// The balance's public shares
    pub public_shares: Vec<BigDecimal>,
    /// The mint of the token in the balance
    pub mint: String,
    /// The address of the balance's owner
    pub owner_address: String,
    /// The address to which the relayer fees are paid
    pub relayer_fee_recipient: String,
    /// A one-time signing authority for the balance
    pub one_time_authority: String,
    /// The protocol fee owed on this balance
    pub protocol_fee: BigDecimal,
    /// The relayer fee owed on this balance
    pub relayer_fee: BigDecimal,
    /// The amount of the token in the balance
    pub amount: BigDecimal,
    /// The ID of the account owning the balance
    pub account_id: Uuid,
    /// Whether the balance is active
    pub active: bool,
}

impl From<BalanceStateObject> for BalanceModel {
    fn from(value: BalanceStateObject) -> Self {
        let nullifier_bigdecimal = scalar_to_bigdecimal(value.balance.compute_nullifier());

        let BalanceStateObject {
            balance:
                DarkpoolStateBalance {
                    inner:
                        DarkpoolBalance {
                            mint,
                            owner,
                            relayer_fee_recipient,
                            authority,
                            relayer_fee_balance,
                            protocol_fee_balance,
                            amount,
                        },
                    recovery_stream,
                    share_stream,
                    public_share,
                },
            account_id,
            active,
        } = value;

        let mint_string = mint.to_string();
        let owner_address_string = owner.to_string();
        let relayer_fee_recipient_string = relayer_fee_recipient.to_string();
        let one_time_authority_string = serde_json::to_string(&authority)
            .expect("SchnorrPublicKey serialization should not fail");

        let protocol_fee_bigdecimal = protocol_fee_balance.into();
        let relayer_fee_bigdecimal = relayer_fee_balance.into();
        let amount_bigdecimal = amount.into();

        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream.seed);
        // The balance's version is the previous index in the recovery stream
        let version_bigdecimal = (recovery_stream.index - 1) as i64;

        let share_stream_seed_bigdecimal = scalar_to_bigdecimal(share_stream.seed);
        let share_stream_index_bigdecimal = share_stream.index as i64;

        let public_shares_bigdecimals =
            public_share.to_scalars().into_iter().map(scalar_to_bigdecimal).collect();

        BalanceModel {
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            version: version_bigdecimal,
            share_stream_seed: share_stream_seed_bigdecimal,
            share_stream_index: share_stream_index_bigdecimal,
            nullifier: nullifier_bigdecimal,
            public_shares: public_shares_bigdecimals,
            account_id,
            active,
            mint: mint_string,
            owner_address: owner_address_string,
            relayer_fee_recipient: relayer_fee_recipient_string,
            one_time_authority: one_time_authority_string,
            protocol_fee: protocol_fee_bigdecimal,
            relayer_fee: relayer_fee_bigdecimal,
            amount: amount_bigdecimal,
        }
    }
}

impl From<BalanceModel> for BalanceStateObject {
    fn from(value: BalanceModel) -> Self {
        let BalanceModel {
            recovery_stream_seed,
            version,
            share_stream_seed,
            share_stream_index,
            // We don't need the nullifier, it can be computed from the circuit type
            nullifier: _nullifier,
            public_shares,
            mint,
            owner_address,
            relayer_fee_recipient,
            one_time_authority,
            protocol_fee,
            relayer_fee,
            amount,
            account_id,
            active,
        } = value;

        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);
        let version_u64 = version as u64;
        // The balance's recovery stream index is always one more than the version
        let recovery_stream_index = version_u64 + 1;
        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed_scalar);
        recovery_stream.index = recovery_stream_index;

        let share_stream_seed_scalar = bigdecimal_to_scalar(share_stream_seed);
        let share_stream_index_u64 = share_stream_index as u64;

        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed_scalar);
        share_stream.index = share_stream_index_u64;

        let public_shares_scalars = DarkpoolBalanceShare::from_scalars(
            &mut public_shares.into_iter().map(bigdecimal_to_scalar),
        );

        let mint_address = Address::from_str(&mint).expect("Mint must be a valid address");
        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let relayer_fee_recipient_address = Address::from_str(&relayer_fee_recipient)
            .expect("Relayer fee recipient must be a valid address");

        let authority_key: SchnorrPublicKey = serde_json::from_str(&one_time_authority)
            .expect("Authority must be a valid SchnorrPublicKey JSON");

        let relayer_fee_u128 =
            relayer_fee.to_u128().expect("Relayer fee cannot be converted to u128");

        let protocol_fee_u128 =
            protocol_fee.to_u128().expect("Protocol fee cannot be converted to u128");

        let amount_u128 = amount.to_u128().expect("Amount cannot be converted to u128");

        BalanceStateObject {
            balance: DarkpoolStateBalance {
                inner: DarkpoolBalance {
                    mint: mint_address,
                    owner: owner_address_address,
                    relayer_fee_recipient: relayer_fee_recipient_address,
                    authority: authority_key,
                    relayer_fee_balance: relayer_fee_u128,
                    protocol_fee_balance: protocol_fee_u128,
                    amount: amount_u128,
                },
                recovery_stream,
                share_stream,
                public_share: public_shares_scalars,
            },
            account_id,
            active,
        }
    }
}
