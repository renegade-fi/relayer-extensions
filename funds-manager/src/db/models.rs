#![allow(missing_docs)]
#![allow(trivial_bounds)]

use bigdecimal::BigDecimal;
use diesel::prelude::*;
use num_bigint::BigInt;
use renegade_circuit_types::note::Note;
use renegade_crypto::fields::scalar_to_bigint;
use renegade_util::hex::{biguint_to_hex_addr, jubjub_to_hex_string};
use uuid::Uuid;

use crate::db::schema::fees;

/// A fee that has been indexed by the indexer
#[derive(Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::fees)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub struct Fee {
    pub id: i32,
    pub tx_hash: String,
    pub mint: String,
    pub amount: BigDecimal,
    pub blinder: BigDecimal,
    pub receiver: String,
    pub redeemed: bool,
}

/// A new fee inserted into the database
#[derive(Insertable)]
#[diesel(table_name = fees)]
pub struct NewFee {
    pub tx_hash: String,
    pub mint: String,
    pub amount: BigDecimal,
    pub blinder: BigDecimal,
    pub receiver: String,
}

impl NewFee {
    /// Construct a fee from a note
    pub fn new_from_note(note: &Note, tx_hash: String) -> Self {
        let mint = biguint_to_hex_addr(&note.mint);
        let amount = BigInt::from(note.amount).into();
        let blinder = scalar_to_bigint(&note.blinder).into();
        let receiver = jubjub_to_hex_string(&note.receiver);

        NewFee { tx_hash, mint, amount, blinder, receiver }
    }
}

/// Metadata information maintained by the indexer
#[derive(Clone, Queryable, Selectable)]
#[diesel(table_name = crate::db::schema::indexing_metadata)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub struct Metadata {
    pub key: String,
    pub value: String,
}

/// A metadata entry for a wallet managed by the indexer
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::wallets)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub struct WalletMetadata {
    pub id: Uuid,
    pub mints: Vec<Option<String>>,
    pub secret_id: String,
}

impl WalletMetadata {
    /// Construct a new wallet metadata entry
    pub fn empty(id: Uuid, secret_id: String) -> Self {
        WalletMetadata { id, mints: vec![], secret_id }
    }
}
