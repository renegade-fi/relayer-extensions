//! Type bindings for the indexer's database table records

use std::{io::Write, str::FromStr};

use alloy::primitives::Address;
use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{
    Selectable,
    deserialize::{self, FromSql, FromSqlRow},
    expression::AsExpression,
    pg::{Pg, PgValue},
    prelude::{AsChangeset, Insertable, Queryable},
    serialize::{self, IsNull, Output, ToSql},
};
use renegade_circuit_types::{balance::Balance, csprng::PoseidonCSPRNG, intent::Intent};
use uuid::Uuid;

use crate::{
    crypto_mocks::{
        recovery_stream::create_recovery_seed_csprng, share_stream::create_share_seed_csprng,
    },
    db::{
        schema::sql_types::ObjectType as ObjectTypeSqlType,
        utils::{
            bigdecimal_to_fixed_point, bigdecimal_to_scalar, fixed_point_to_bigdecimal,
            scalar_to_bigdecimal,
        },
    },
    types::{
        BalanceStateObject, ExpectedStateObject, GenericStateObject, IntentStateObject,
        MasterViewSeed, StateObjectType,
    },
};

// ----------------------------
// | Custom SQL Type Bindings |
// ----------------------------

// === Object Type ===

/// The state of an order
#[derive(Debug, Clone, Copy, PartialEq, FromSqlRow, AsExpression, Eq)]
#[diesel(sql_type = ObjectTypeSqlType)]
pub enum DbStateObjectType {
    /// An intent state object
    Intent,
    /// A balance state object
    Balance,
}

impl ToSql<ObjectTypeSqlType, Pg> for DbStateObjectType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        match *self {
            DbStateObjectType::Intent => out.write_all(b"intent")?,
            DbStateObjectType::Balance => out.write_all(b"balance")?,
        }
        Ok(IsNull::No)
    }
}

impl FromSql<ObjectTypeSqlType, Pg> for DbStateObjectType {
    fn from_sql(bytes: PgValue<'_>) -> deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"intent" => Ok(DbStateObjectType::Intent),
            b"balance" => Ok(DbStateObjectType::Balance),
            _ => Err("Unrecognized enum variant for object_type".into()),
        }
    }
}

impl From<StateObjectType> for DbStateObjectType {
    fn from(value: StateObjectType) -> Self {
        match value {
            StateObjectType::Intent => DbStateObjectType::Intent,
            StateObjectType::Balance => DbStateObjectType::Balance,
        }
    }
}

impl From<DbStateObjectType> for StateObjectType {
    fn from(value: DbStateObjectType) -> Self {
        match value {
            DbStateObjectType::Intent => StateObjectType::Intent,
            DbStateObjectType::Balance => StateObjectType::Balance,
        }
    }
}

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
    pub recovery_seed_csprng_index: BigDecimal,
    /// The index of the share seed CSPRNG
    pub share_seed_csprng_index: BigDecimal,
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

        let recovery_seed_csprng_index_bigdecimal = recovery_seed_csprng.index.into();
        let share_seed_csprng_index_bigdecimal = share_seed_csprng.index.into();

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

        let recovery_seed_csprng_index_u64 = recovery_seed_csprng_index
            .to_u64()
            .expect("Recovery seed CSPRNG index cannot be converted to u64");

        let share_seed_csprng_index_u64 = share_seed_csprng_index
            .to_u64()
            .expect("Share seed CSPRNG index cannot be converted to u64");

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
    /// The expected nullifier
    pub nullifier: BigDecimal,
    /// The ID of the account owning the state object associated with the
    /// nullifier
    pub account_id: Uuid,
    /// The address of the owner of the state object associated with the
    /// nullifier
    pub owner_address: String,
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
            nullifier,
            account_id,
            owner_address,
            recovery_stream,
            share_stream,
        } = value;

        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream.seed);
        let share_stream_seed_bigdecimal = scalar_to_bigdecimal(share_stream.seed);
        let owner_address_string = owner_address.to_string();

        ExpectedStateObjectModel {
            nullifier: nullifier_bigdecimal,
            account_id,
            owner_address: owner_address_string,
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            share_stream_seed: share_stream_seed_bigdecimal,
        }
    }
}

impl From<ExpectedStateObjectModel> for ExpectedStateObject {
    fn from(value: ExpectedStateObjectModel) -> Self {
        let ExpectedStateObjectModel {
            account_id,
            owner_address,
            recovery_stream_seed,
            share_stream_seed,
            ..
        } = value;

        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);
        let share_stream_seed_scalar = bigdecimal_to_scalar(share_stream_seed);
        let owner_address_alloy =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        ExpectedStateObject::new(
            account_id,
            owner_address_alloy,
            recovery_stream_seed_scalar,
            share_stream_seed_scalar,
        )
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
    pub block_number: BigDecimal,
}

// === Generic State Objects Table ===

/// A generic state object record
#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = crate::db::schema::generic_state_objects)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct GenericStateObjectModel {
    /// The object's recovery stream seed
    pub recovery_stream_seed: BigDecimal,
    /// The ID of the account owning the state object
    pub account_id: Uuid,
    /// Whether the object is active
    pub active: bool,
    /// The type of the object
    pub object_type: DbStateObjectType,
    /// The object's current (unspent) nullifier
    pub nullifier: BigDecimal,
    /// The object's current version
    pub version: BigDecimal,
    /// The object's share stream seed
    pub share_stream_seed: BigDecimal,
    /// The current index of the object's share stream
    pub share_stream_index: BigDecimal,
    /// The address of the object's owner
    pub owner_address: String,
    /// The public shares of the object
    pub public_shares: Vec<BigDecimal>,
    /// The private shares of the object
    pub private_shares: Vec<BigDecimal>,
}

impl From<GenericStateObject> for GenericStateObjectModel {
    fn from(value: GenericStateObject) -> Self {
        let GenericStateObject {
            recovery_stream,
            account_id,
            active,
            object_type,
            nullifier,
            share_stream,
            owner_address,
            public_shares,
            private_shares,
        } = value;

        let db_object_type = object_type.into();

        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream.seed);
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let share_stream_seed_bigdecimal = scalar_to_bigdecimal(share_stream.seed);

        // The object's current version is the previous index in the recovery stream
        let version_bigdecimal = (recovery_stream.index - 1).into();
        let share_stream_index_bigdecimal = share_stream.index.into();

        let owner_address_string = owner_address.to_string();

        let public_shares_bigdecimals =
            public_shares.into_iter().map(scalar_to_bigdecimal).collect();

        let private_shares_bigdecimals =
            private_shares.into_iter().map(scalar_to_bigdecimal).collect();

        GenericStateObjectModel {
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            account_id,
            active,
            object_type: db_object_type,
            nullifier: nullifier_bigdecimal,
            version: version_bigdecimal,
            share_stream_seed: share_stream_seed_bigdecimal,
            share_stream_index: share_stream_index_bigdecimal,
            owner_address: owner_address_string,
            public_shares: public_shares_bigdecimals,
            private_shares: private_shares_bigdecimals,
        }
    }
}

impl From<GenericStateObjectModel> for GenericStateObject {
    fn from(value: GenericStateObjectModel) -> Self {
        let GenericStateObjectModel {
            recovery_stream_seed,
            account_id,
            active,
            object_type,
            nullifier,
            version,
            share_stream_seed,
            share_stream_index,
            owner_address,
            public_shares,
            private_shares,
        } = value;

        let object_type_state = object_type.into();

        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);
        let nullifier_scalar = bigdecimal_to_scalar(nullifier);
        let share_stream_seed_scalar = bigdecimal_to_scalar(share_stream_seed);

        // The object's version is given by the *previous* index in the recovery stream
        let version_u64 = version.to_u64().expect("Version cannot be converted to u64");
        let recovery_stream_index = version_u64 + 1;

        let share_stream_index_u64 =
            share_stream_index.to_u64().expect("Share stream index cannot be converted to u64");

        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let public_shares_scalars = public_shares.into_iter().map(bigdecimal_to_scalar).collect();
        let private_shares_scalars = private_shares.into_iter().map(bigdecimal_to_scalar).collect();

        let mut recovery_stream = PoseidonCSPRNG::new(recovery_stream_seed_scalar);

        recovery_stream.index = recovery_stream_index;

        let mut share_stream = PoseidonCSPRNG::new(share_stream_seed_scalar);
        share_stream.index = share_stream_index_u64;

        GenericStateObject {
            recovery_stream,
            account_id,
            active,
            object_type: object_type_state,
            nullifier: nullifier_scalar,
            share_stream,
            owner_address: owner_address_address,
            public_shares: public_shares_scalars,
            private_shares: private_shares_scalars,
        }
    }
}

// === Intents Table ===

/// An intent record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::intents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct IntentModel {
    /// The intent's recovery stream seed
    pub recovery_stream_seed: BigDecimal,
    /// The ID of the account owning the intent
    pub account_id: Uuid,
    /// Whether the intent is active
    pub active: bool,
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
        let IntentStateObject {
            intent: Intent { in_token, out_token, owner, min_price, amount_in },
            recovery_stream_seed,
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

        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);
        let min_fill_size_bigdecimal = min_fill_size.into();

        IntentModel {
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
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

        let input_mint_address =
            Address::from_str(&input_mint).expect("Input mint must be a valid address");
        let output_mint_address =
            Address::from_str(&output_mint).expect("Output mint must be a valid address");
        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");
        let min_price_fixed_point = bigdecimal_to_fixed_point(min_price);
        let input_amount_u128 =
            input_amount.to_u128().expect("Input amount cannot be converted to u128");

        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);
        let min_fill_size_u128 =
            min_fill_size.to_u128().expect("Min fill size cannot be converted to u128");

        IntentStateObject {
            intent: Intent {
                in_token: input_mint_address,
                out_token: output_mint_address,
                owner: owner_address_address,
                min_price: min_price_fixed_point,
                amount_in: input_amount_u128,
            },
            recovery_stream_seed: recovery_stream_seed_scalar,
            account_id,
            active,
            matching_pool,
            allow_external_matches,
            min_fill_size: min_fill_size_u128,
            precompute_cancellation_proof,
        }
    }
}

/// A changeset for updating the core fields of an intent, which are committed
/// to onchain
#[derive(AsChangeset)]
#[diesel(table_name = crate::db::schema::intents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct IntentCoreChangeset {
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
}

impl From<Intent> for IntentCoreChangeset {
    fn from(value: Intent) -> Self {
        let Intent { in_token, out_token, owner, min_price, amount_in } = value;

        let input_mint_string = in_token.to_string();
        let output_mint_string = out_token.to_string();
        let owner_address_string = owner.to_string();

        let min_price_bigdecimal = fixed_point_to_bigdecimal(min_price);
        let input_amount_bigdecimal = amount_in.into();

        IntentCoreChangeset {
            input_mint: input_mint_string,
            output_mint: output_mint_string,
            owner_address: owner_address_string,
            min_price: min_price_bigdecimal,
            input_amount: input_amount_bigdecimal,
        }
    }
}

// === Balances Table ===

/// A balance record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::balances)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BalanceModel {
    /// The balance's recovery stream seed
    pub recovery_stream_seed: BigDecimal,
    /// The ID of the account owning the balance
    pub account_id: Uuid,
    /// Whether the balance is active
    pub active: bool,
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
    /// Whether public fills are allowed on this balance
    pub allow_public_fills: bool,
}

impl From<BalanceStateObject> for BalanceModel {
    fn from(value: BalanceStateObject) -> Self {
        let BalanceStateObject {
            balance:
                Balance {
                    mint,
                    owner,
                    relayer_fee_recipient,
                    one_time_authority,
                    relayer_fee_balance,
                    protocol_fee_balance,
                    amount,
                },
            recovery_stream_seed,
            account_id,
            active,
            allow_public_fills,
        } = value;

        let recovery_stream_seed_bigdecimal = scalar_to_bigdecimal(recovery_stream_seed);
        let mint_string = mint.to_string();
        let owner_address_string = owner.to_string();
        let relayer_fee_recipient_string = relayer_fee_recipient.to_string();
        let one_time_authority_string = one_time_authority.to_string();
        let protocol_fee_bigdecimal = protocol_fee_balance.into();
        let relayer_fee_bigdecimal = relayer_fee_balance.into();
        let amount_bigdecimal = amount.into();

        BalanceModel {
            recovery_stream_seed: recovery_stream_seed_bigdecimal,
            account_id,
            active,
            mint: mint_string,
            owner_address: owner_address_string,
            relayer_fee_recipient: relayer_fee_recipient_string,
            one_time_authority: one_time_authority_string,
            protocol_fee: protocol_fee_bigdecimal,
            relayer_fee: relayer_fee_bigdecimal,
            amount: amount_bigdecimal,
            allow_public_fills,
        }
    }
}

impl From<BalanceModel> for BalanceStateObject {
    fn from(value: BalanceModel) -> Self {
        let BalanceModel {
            recovery_stream_seed,
            account_id,
            active,
            mint,
            owner_address,
            relayer_fee_recipient,
            one_time_authority,
            protocol_fee,
            relayer_fee,
            amount,
            allow_public_fills,
        } = value;

        let mint_address = Address::from_str(&mint).expect("Mint must be a valid address");
        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let relayer_fee_recipient_address = Address::from_str(&relayer_fee_recipient)
            .expect("Relayer fee recipient must be a valid address");

        let one_time_authority_address = Address::from_str(&one_time_authority)
            .expect("One time authority must be a valid address");

        let relayer_fee_u128 =
            relayer_fee.to_u128().expect("Relayer fee cannot be converted to u128");

        let protocol_fee_u128 =
            protocol_fee.to_u128().expect("Protocol fee cannot be converted to u128");

        let amount_u128 = amount.to_u128().expect("Amount cannot be converted to u128");

        let recovery_stream_seed_scalar = bigdecimal_to_scalar(recovery_stream_seed);

        BalanceStateObject {
            balance: Balance {
                mint: mint_address,
                owner: owner_address_address,
                relayer_fee_recipient: relayer_fee_recipient_address,
                one_time_authority: one_time_authority_address,
                relayer_fee_balance: relayer_fee_u128,
                protocol_fee_balance: protocol_fee_u128,
                amount: amount_u128,
            },
            recovery_stream_seed: recovery_stream_seed_scalar,
            account_id,
            active,
            allow_public_fills,
        }
    }
}

/// A changeset for updating the core fields of a balance, which are committed
/// to onchain
#[derive(AsChangeset)]
#[diesel(table_name = crate::db::schema::balances)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BalanceCoreChangeset {
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
}

impl From<Balance> for BalanceCoreChangeset {
    fn from(value: Balance) -> Self {
        let Balance {
            mint,
            owner,
            relayer_fee_recipient,
            one_time_authority,
            protocol_fee_balance,
            relayer_fee_balance,
            amount,
        } = value;

        let mint_string = mint.to_string();
        let owner_address_string = owner.to_string();
        let relayer_fee_recipient_string = relayer_fee_recipient.to_string();
        let one_time_authority_string = one_time_authority.to_string();
        let protocol_fee_bigdecimal = protocol_fee_balance.into();
        let relayer_fee_bigdecimal = relayer_fee_balance.into();
        let amount_bigdecimal = amount.into();

        BalanceCoreChangeset {
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
