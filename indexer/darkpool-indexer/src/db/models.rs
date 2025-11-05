//! Type bindings for the indexer's database table records

use std::{io::Write, str::FromStr};

use alloy_primitives::Address;
use bigdecimal::{BigDecimal, ToPrimitive};
use diesel::{
    Selectable,
    deserialize::{self, FromSql, FromSqlRow},
    expression::AsExpression,
    pg::{Pg, PgValue},
    prelude::{Insertable, Queryable},
    serialize::{self, IsNull, Output, ToSql},
};
use uuid::Uuid;

use crate::{
    db::{
        schema::sql_types::ObjectType as ObjectTypeSqlType,
        utils::{bigdecimal_to_scalar, scalar_to_bigdecimal},
    },
    types::{ExpectedStateObject, GenericStateObject, MasterViewSeed, StateObjectType},
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
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::master_view_seeds)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MasterViewSeedModel {
    /// The ID of the seed owner's account
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: String,
    /// The master view seed
    pub seed: BigDecimal,
}

impl From<MasterViewSeed> for MasterViewSeedModel {
    fn from(value: MasterViewSeed) -> Self {
        let MasterViewSeed { account_id, owner_address, seed } = value;

        let seed_bigdecimal = scalar_to_bigdecimal(seed);
        let owner_address_string = owner_address.to_string();

        MasterViewSeedModel {
            account_id,
            owner_address: owner_address_string,
            seed: seed_bigdecimal,
        }
    }
}

impl From<MasterViewSeedModel> for MasterViewSeed {
    fn from(value: MasterViewSeedModel) -> Self {
        let MasterViewSeedModel { account_id, owner_address, seed } = value;

        let seed_scalar = bigdecimal_to_scalar(seed);
        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        MasterViewSeed { account_id, owner_address: owner_address_address, seed: seed_scalar }
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
    /// The identifier stream seed of the state object associated with the
    /// nullifier
    pub identifier_seed: BigDecimal,
    /// The encryption cipher seed of the state object associated with the
    /// nullifier
    pub encryption_seed: BigDecimal,
}

impl From<ExpectedStateObject> for ExpectedStateObjectModel {
    fn from(value: ExpectedStateObject) -> Self {
        let ExpectedStateObject {
            nullifier,
            account_id,
            owner_address,
            identifier_seed,
            encryption_seed,
        } = value;

        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);
        let encryption_seed_bigdecimal = scalar_to_bigdecimal(encryption_seed);
        let owner_address_string = owner_address.to_string();

        ExpectedStateObjectModel {
            nullifier: nullifier_bigdecimal,
            account_id,
            owner_address: owner_address_string,
            identifier_seed: identifier_seed_bigdecimal,
            encryption_seed: encryption_seed_bigdecimal,
        }
    }
}

impl From<ExpectedStateObjectModel> for ExpectedStateObject {
    fn from(value: ExpectedStateObjectModel) -> Self {
        let ExpectedStateObjectModel {
            nullifier,
            account_id,
            owner_address,
            identifier_seed,
            encryption_seed,
        } = value;

        let nullifier_scalar = bigdecimal_to_scalar(nullifier);
        let identifier_seed_scalar = bigdecimal_to_scalar(identifier_seed);
        let encryption_seed_scalar = bigdecimal_to_scalar(encryption_seed);
        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        ExpectedStateObject {
            nullifier: nullifier_scalar,
            account_id,
            owner_address: owner_address_address,
            identifier_seed: identifier_seed_scalar,
            encryption_seed: encryption_seed_scalar,
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
    pub block_number: BigDecimal,
}

// === Generic State Objects Table ===

/// A generic state object record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::generic_state_objects)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct GenericStateObjectModel {
    /// The object's identifier stream seed
    pub identifier_seed: BigDecimal,
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
    /// The object's encryption cipher seed
    pub encryption_seed: BigDecimal,
    /// The current index of the object's encryption cipher
    pub encryption_cipher_index: BigDecimal,
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
            identifier_seed,
            account_id,
            active,
            object_type,
            nullifier,
            version,
            encryption_seed,
            encryption_cipher_index,
            owner_address,
            public_shares,
            private_shares,
        } = value;

        let db_object_type = object_type.into();

        let identifier_seed_bigdecimal = scalar_to_bigdecimal(identifier_seed);
        let nullifier_bigdecimal = scalar_to_bigdecimal(nullifier);
        let encryption_seed_bigdecimal = scalar_to_bigdecimal(encryption_seed);

        let version_bigdecimal = (version as u64).into();
        let encryption_cipher_index_bigdecimal = (encryption_cipher_index as u64).into();

        let owner_address_string = owner_address.to_string();

        let public_shares_bigdecimals =
            public_shares.into_iter().map(scalar_to_bigdecimal).collect();

        let private_shares_bigdecimals =
            private_shares.into_iter().map(scalar_to_bigdecimal).collect();

        GenericStateObjectModel {
            identifier_seed: identifier_seed_bigdecimal,
            account_id,
            active,
            object_type: db_object_type,
            nullifier: nullifier_bigdecimal,
            version: version_bigdecimal,
            encryption_seed: encryption_seed_bigdecimal,
            encryption_cipher_index: encryption_cipher_index_bigdecimal,
            owner_address: owner_address_string,
            public_shares: public_shares_bigdecimals,
            private_shares: private_shares_bigdecimals,
        }
    }
}

impl From<GenericStateObjectModel> for GenericStateObject {
    fn from(value: GenericStateObjectModel) -> Self {
        let GenericStateObjectModel {
            identifier_seed,
            account_id,
            active,
            object_type,
            nullifier,
            version,
            encryption_seed,
            encryption_cipher_index,
            owner_address,
            public_shares,
            private_shares,
        } = value;

        let object_type_state = object_type.into();

        let identifier_seed_scalar = bigdecimal_to_scalar(identifier_seed);
        let nullifier_scalar = bigdecimal_to_scalar(nullifier);
        let encryption_seed_scalar = bigdecimal_to_scalar(encryption_seed);

        let version_usize = version.to_usize().expect("Version cannot be converted to usize");
        let encryption_cipher_index_usize = encryption_cipher_index
            .to_usize()
            .expect("Encryption cipher index cannot be converted to usize");

        let owner_address_address =
            Address::from_str(&owner_address).expect("Owner address must be a valid address");

        let public_shares_scalars = public_shares.into_iter().map(bigdecimal_to_scalar).collect();
        let private_shares_scalars = private_shares.into_iter().map(bigdecimal_to_scalar).collect();

        GenericStateObject {
            identifier_seed: identifier_seed_scalar,
            account_id,
            active,
            object_type: object_type_state,
            nullifier: nullifier_scalar,
            version: version_usize,
            encryption_seed: encryption_seed_scalar,
            encryption_cipher_index: encryption_cipher_index_usize,
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
    /// The intent's identifier stream seed
    pub identifier_seed: BigDecimal,
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

// === Balances Table ===

/// A balance record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::balances)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct BalanceModel {
    /// The balance's identifier stream seed
    pub identifier_seed: BigDecimal,
    /// The ID of the account owning the balance
    pub account_id: Uuid,
    /// Whether the balance is active
    pub active: bool,
    /// The mint of the token in the balance
    pub mint: String,
    /// The address of the balance's owner
    pub owner_address: String,
    /// The one-time key used for authorizing fills capitalized by this balance
    pub one_time_key: String,
    /// The protocol fee owed on this balance
    pub protocol_fee: BigDecimal,
    /// The relayer fee owed on this balance
    pub relayer_fee: BigDecimal,
    /// The amount of the token in the balance
    pub amount: BigDecimal,
    /// Whether public fills are allowed on this balance
    pub allow_public_fills: bool,
}
