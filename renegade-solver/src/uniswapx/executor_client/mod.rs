//! The definition of the executor client, which holds the configuration
//! details, along with a lower-level handle for the executor smart contract
use crate::cli::{chain_to_chain_id, Cli};
use crate::uniswapx::executor_client::errors::{ExecutorConfigError, ExecutorError};

use alloy::consensus::SignableTransaction;
use alloy::network::TxSignerSync;
use alloy::rpc::types::TransactionRequest;
use alloy::{
    providers::{DynProvider, Provider, ProviderBuilder, WsConnect},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{Address, Bytes, ChainId, TxHash};
use renegade_solidity_abi::IDarkpool::IDarkpoolInstance as ExecutorInstance;
use renegade_util::err_str;
use std::str::FromStr;

mod contract_interaction;
pub mod errors;

// -------------
// | Constants |
// -------------

/// A type alias for the RPC client, which is an alloy middleware stack that
/// includes a signer derived from a raw private key, and a provider that
/// connects to the RPC endpoint over HTTP.
pub type ExecutorProvider = DynProvider;

/// A configuration struct for the executor client, consists of relevant
/// contract addresses, and endpoint for setting up an RPC client, and a private
/// key for signing transactions.
pub struct ExecutorConfig {
    /// The address of the executor proxy contract.
    ///
    /// This is the main entrypoint to interaction with the executor.
    pub contract_address: String,
    /// WebSocket endpoint for real-time block monitoring
    pub rpc_websocket_url: String,
    /// The signer to use for signing transactions
    pub signer: PrivateKeySigner,
}

// ----------
// | Client |
// ----------

impl ExecutorConfig {
    /// Creates a new configuration
    pub fn new(
        contract_address: String,
        rpc_websocket_url: String,
        signer: PrivateKeySigner,
    ) -> Self {
        Self { contract_address, rpc_websocket_url, signer }
    }

    /// Create a WebSocket provider
    async fn get_ws_provider(&self) -> Result<ExecutorProvider, ExecutorConfigError> {
        let conn = WsConnect::new(self.rpc_websocket_url.clone());
        let provider = ProviderBuilder::new()
            .wallet(self.signer.clone())
            .connect_ws(conn)
            .await
            .map_err(err_str!(ExecutorConfigError::RpcClientInitialization))?;
        Ok(DynProvider::new(provider))
    }

    /// Parses the executor contract address
    fn get_contract_address(&self) -> Result<Address, ExecutorConfigError> {
        Address::from_str(&self.contract_address)
            .map_err(err_str!(ExecutorConfigError::AddressParsing))
    }
}

/// The executor client, which provides a higher-level interface to the executor
/// contract for Renegade-specific access patterns.
pub struct ExecutorClient {
    /// The executor contract instance
    contract: ExecutorInstance<ExecutorProvider>,
    /// The shared provider used for submissions and reads/subscriptions
    provider: ExecutorProvider,
    /// The signer for the executor client
    signer: PrivateKeySigner,
    /// The chain ID
    pub chain_id: ChainId,
}

impl ExecutorClient {
    /// Creates a new ExecutorClient from CLI configuration
    pub async fn new(cli: &Cli) -> Result<Self, ExecutorConfigError> {
        // Parse the private key
        let signer = PrivateKeySigner::from_str(&cli.private_key)
            .map_err(err_str!(ExecutorConfigError::AddressParsing))?;

        // Create the configuration
        // Use explicit WebSocket URL from CLI
        let rpc_websocket_url = cli.rpc_websocket_url.clone();

        let config =
            ExecutorConfig::new(cli.contract_address.clone(), rpc_websocket_url, signer.clone());

        // Build the shared provider (WebSocket)
        let provider = config.get_ws_provider().await?;

        let contract_address = config.get_contract_address()?;
        // Bind the contract to the provider
        let contract = ExecutorInstance::new(contract_address, provider.clone());

        let chain_id = chain_to_chain_id(&cli.chain_id);

        Ok(Self { contract, provider, signer, chain_id })
    }

    /// Get a clone of the shared provider
    pub fn provider(&self) -> ExecutorProvider {
        self.provider.clone()
    }

    /// Sends a raw signed transaction. Returns the tx hash.
    pub async fn send_raw(&self, raw: Bytes) -> eyre::Result<TxHash> {
        let pending = self.provider.send_raw_transaction(&raw).await?;
        let hash = pending.tx_hash().to_owned();
        Ok(hash)
    }

    /// Sign a fully qualified transaction
    ///
    /// Returns the raw bytes of the signed transaction and the hash
    pub fn sign_transaction(
        &self,
        req: TransactionRequest,
    ) -> Result<(Bytes, TxHash), ExecutorError> {
        let mut tx = req.build_1559().unwrap();

        // Use the signer to sign the transaction and get the hash
        let signature = self.signer.sign_transaction_sync(&mut tx)?;
        let signed_tx = tx.into_signed(signature);
        let tx_hash = signed_tx.hash().to_owned();

        // Get the raw bytes of the signed transaction
        let mut raw_tx_bytes = Vec::new();
        signed_tx.eip2718_encode(&mut raw_tx_bytes);

        Ok((raw_tx_bytes.into(), tx_hash))
    }
}

impl Clone for ExecutorClient {
    fn clone(&self) -> Self {
        Self {
            contract: self.contract.clone(),
            provider: self.provider.clone(),
            signer: self.signer.clone(),
            chain_id: self.chain_id,
        }
    }
}
