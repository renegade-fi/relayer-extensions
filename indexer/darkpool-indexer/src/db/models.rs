//! Type bindings for the indexer's database table records

use std::io::Write;

use bigdecimal::BigDecimal;
use diesel::{
    Selectable,
    deserialize::{self, FromSql, FromSqlRow},
    expression::AsExpression,
    pg::{Pg, PgValue},
    prelude::{Insertable, Queryable},
    serialize::{self, IsNull, Output, ToSql},
};
use uuid::Uuid;

use crate::db::schema::sql_types::ObjectType as ObjectTypeSqlType;

// ----------------------------
// | Custom SQL Type Bindings |
// ----------------------------

// === Object Type ===

/// The state of an order
#[derive(Debug, Clone, Copy, PartialEq, FromSqlRow, AsExpression, Eq)]
#[diesel(sql_type = ObjectTypeSqlType)]
pub enum ObjectType {
    /// An intent state object
    Intent,
    /// A balance state object
    Balance,
}

impl ToSql<ObjectTypeSqlType, Pg> for ObjectType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> serialize::Result {
        match *self {
            ObjectType::Intent => out.write_all(b"intent")?,
            ObjectType::Balance => out.write_all(b"balance")?,
        }
        Ok(IsNull::No)
    }
}

impl FromSql<ObjectTypeSqlType, Pg> for ObjectType {
    fn from_sql(bytes: PgValue<'_>) -> deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"intent" => Ok(ObjectType::Intent),
            b"balance" => Ok(ObjectType::Balance),
            _ => Err("Unrecognized enum variant for object_type".into()),
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
pub struct MasterViewSeed {
    /// The ID of the seed owner's account
    pub account_id: Uuid,
    /// The address of the seed's owner
    pub owner_address: String,
    /// The master view seed
    pub seed: BigDecimal,
}

// === Expected Nullifiers Table ===

/// An expected nullifier record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::expected_nullifiers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ExpectedNullifier {
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

// === Processed Nullifiers Table ===

/// A processed nullifier record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::processed_nullifiers)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ProcessedNullifier {
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
pub struct GenericStateObject {
    /// The object's identifier stream seed
    pub identifier_seed: BigDecimal,
    /// The ID of the account owning the state object
    pub account_id: Uuid,
    /// Whether the object is active
    pub active: bool,
    /// The type of the object
    pub object_type: ObjectType,
    /// The object's current (unspent) nullifier
    pub nullifier: BigDecimal,
    /// The object's current version
    pub version: BigDecimal,
    /// The object's encryption cipher seed
    pub encryption_seed: BigDecimal,
    /// The address of the object's owner
    pub owner_address: String,
    /// The public shares of the object
    pub public_shares: Vec<BigDecimal>,
    /// The private shares of the object
    pub private_shares: Vec<BigDecimal>,
}

// === Intents Table ===

/// An intent record
#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::intents)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Intent {
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
pub struct Balance {
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
