#![allow(missing_docs)]
#![allow(trivial_bounds)]

use std::{fmt::Display, str::FromStr, time::SystemTime};

use bigdecimal::BigDecimal;
use diesel::prelude::*;
use num_bigint::BigInt;
use renegade_circuit_types::note::Note;
use renegade_common::types::chain::Chain;
use renegade_crypto::fields::scalar_to_bigint;
use renegade_util::hex::{biguint_to_hex_addr, jubjub_to_hex_string};
use uuid::Uuid;

use crate::{cli::Environment, db::schema::fees};

/// Convert a chain to its expected representation in the database,
/// which is agnostic of the environment (testnet, mainnet)
pub fn to_db_chain(chain: Chain) -> String {
    match chain {
        Chain::ArbitrumOne | Chain::ArbitrumSepolia => "arbitrum".to_string(),
        Chain::BaseMainnet | Chain::BaseSepolia => "base".to_string(),
        _ => chain.to_string(),
    }
}

/// Convert a chain as specified in the database to the expected `Chain` enum,
/// accounting for the environment (testnet, mainnet)
pub fn from_db_chain(chain: &str, environment: Environment) -> Chain {
    let arb_chain = match environment {
        Environment::Mainnet => Chain::ArbitrumOne,
        Environment::Testnet => Chain::ArbitrumSepolia,
    };
    let base_chain = match environment {
        Environment::Mainnet => Chain::BaseMainnet,
        Environment::Testnet => Chain::BaseSepolia,
    };

    match chain {
        "arbitrum" => arb_chain,
        "base" => base_chain,
        _ => Chain::from_str(chain).unwrap(),
    }
}

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
    pub chain: String,
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
    pub chain: String,
}

impl NewFee {
    /// Construct a fee from a note
    pub fn new_from_note(note: &Note, tx_hash: String, chain: Chain) -> Self {
        let mint = biguint_to_hex_addr(&note.mint);
        let amount = BigInt::from(note.amount).into();
        let blinder = scalar_to_bigint(&note.blinder).into();
        let receiver = jubjub_to_hex_string(&note.receiver);

        let chain = to_db_chain(chain);

        NewFee { tx_hash, mint, amount, blinder, receiver, chain }
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
    pub chain: String,
}

/// A metadata entry for a wallet managed by the indexer
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::renegade_wallets)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[allow(missing_docs, clippy::missing_docs_in_private_items)]
pub struct RenegadeWalletMetadata {
    pub id: Uuid,
    pub mints: Vec<Option<String>>,
    pub secret_id: String,
    pub chain: String,
}

impl RenegadeWalletMetadata {
    /// Construct a new wallet metadata entry
    pub fn empty(id: Uuid, secret_id: String, chain: Chain) -> Self {
        let chain = to_db_chain(chain);
        RenegadeWalletMetadata { id, mints: vec![], secret_id, chain }
    }
}

/// A hot wallet managed by the custody client
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::hot_wallets)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct HotWallet {
    pub id: Uuid,
    pub secret_id: String,
    pub vault: String,
    pub address: String,
    pub internal_wallet_id: Uuid,
    pub chain: String,
}

impl HotWallet {
    /// Construct a new hot wallet entry
    pub fn new(
        secret_id: String,
        vault: String,
        address: String,
        internal_wallet_id: Uuid,
        chain: Chain,
    ) -> Self {
        let chain = to_db_chain(chain);
        HotWallet { id: Uuid::new_v4(), secret_id, vault, address, internal_wallet_id, chain }
    }
}

/// The status of a gas wallet
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GasWalletStatus {
    /// The gas wallet is active
    Active,
    /// Marked as inactive but not yet transitioned to inactive
    Pending,
    /// The gas wallet is inactive
    Inactive,
}

impl Display for GasWalletStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GasWalletStatus::Active => write!(f, "active"),
            GasWalletStatus::Pending => write!(f, "pending"),
            GasWalletStatus::Inactive => write!(f, "inactive"),
        }
    }
}

impl FromStr for GasWalletStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(GasWalletStatus::Active),
            "pending" => Ok(GasWalletStatus::Pending),
            "inactive" => Ok(GasWalletStatus::Inactive),
            _ => Err(format!("Invalid gas wallet status: {s}")),
        }
    }
}

impl GasWalletStatus {
    /// Get the state resulting from marking the gas wallet as active
    pub fn transition_active(&self) -> Self {
        GasWalletStatus::Active
    }

    /// Get the state resulting from marking the gas wallet as inactive
    pub fn transition_inactive(&self) -> Self {
        match self {
            GasWalletStatus::Active => GasWalletStatus::Pending,
            GasWalletStatus::Pending => GasWalletStatus::Inactive,
            GasWalletStatus::Inactive => GasWalletStatus::Inactive,
        }
    }
}

/// A gas wallet's metadata
#[derive(Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = crate::db::schema::gas_wallets)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct GasWallet {
    pub id: Uuid,
    pub address: String,
    pub peer_id: Option<String>,
    pub status: String,
    pub created_at: SystemTime,
    pub chain: String,
}

impl GasWallet {
    /// Construct a new gas wallet
    pub fn new(address: String, chain: Chain) -> Self {
        let id = Uuid::new_v4();
        let status = GasWalletStatus::Inactive.to_string();
        let chain = to_db_chain(chain);
        GasWallet { id, address, peer_id: None, status, created_at: SystemTime::now(), chain }
    }
}
