//! The indexer handles the indexing and redemption of fee notes

use aws_config::SdkConfig as AwsConfig;
use renegade_circuit_types::elgamal::DecryptionKey;
use renegade_common::types::chain::Chain;
use renegade_darkpool_client::{client::DarkpoolClientInner, traits::DarkpoolImpl};
use renegade_util::err_str;
use renegade_util::hex::jubjub_from_hex_string;
use std::sync::Arc;

use crate::custody_client::CustodyClient;
use crate::db::{DbConn, DbPool};
use crate::error::FundsManagerError;
use crate::relayer_client::RelayerClient;

pub mod fee_balances;
pub mod index_fees;
pub mod queries;
pub mod redeem_fees;

/// The error message for when the chain is not supported
pub(crate) const ERR_UNSUPPORTED_CHAIN: &str = "Unsupported chain";

/// Stores the dependencies needed to index the chain
#[derive(Clone)]
pub(crate) struct Indexer<D: DarkpoolImpl> {
    /// The id of the chain this indexer targets
    pub chain_id: u64,
    /// The chain this indexer targets
    pub chain: Chain,
    /// A client for interacting with the relayer
    pub relayer_client: RelayerClient,
    /// The darkpool client
    pub darkpool_client: DarkpoolClientInner<D>,
    /// The decryption key
    pub decryption_keys: Vec<DecryptionKey>,
    /// The database connection pool
    pub db_pool: Arc<DbPool>,
    /// The AWS config
    pub aws_config: AwsConfig,
    /// The custody client
    pub custody_client: CustodyClient,
}

impl<D: DarkpoolImpl> Indexer<D> {
    /// Constructor
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: u64,
        chain: Chain,
        aws_config: AwsConfig,
        darkpool_client: DarkpoolClientInner<D>,
        decryption_keys: Vec<DecryptionKey>,
        db_pool: Arc<DbPool>,
        relayer_client: RelayerClient,
        custody_client: CustodyClient,
    ) -> Self {
        Indexer {
            chain_id,
            chain,
            darkpool_client,
            decryption_keys,
            db_pool,
            relayer_client,
            aws_config,
            custody_client,
        }
    }

    /// Get the decryption key for a given encryption key, referred to as a
    /// receiver in this context
    pub fn get_key_for_receiver(&self, receiver: &str) -> Option<&DecryptionKey> {
        let key = jubjub_from_hex_string(receiver).ok()?;
        self.decryption_keys.iter().find(|k| k.public_key() == key)
    }

    /// Get a connection from the pool
    pub async fn get_conn(&self) -> Result<DbConn, FundsManagerError> {
        self.db_pool.get().await.map_err(err_str!(FundsManagerError::Db))
    }
}
