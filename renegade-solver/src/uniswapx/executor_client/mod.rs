//! The definition of the executor client, which holds the configuration
//! details, along with a lower-level handle for the executor smart contract
use crate::uniswapx::{
    abis::conversion::u256_to_u128,
    executor_client::errors::{ExecutorConfigError, ExecutorError},
};
use alloy::{
    eips::BlockId,
    providers::{DynProvider, Provider, ProviderBuilder},
    rpc::types::TransactionReceipt,
    signers::local::PrivateKeySigner,
    transports::http::reqwest::Url,
};
use alloy_contract::{CallBuilder, CallDecoder};
use alloy_primitives::Address;
use alloy_primitives::U256;
use renegade_solidity_abi::IDarkpool::IDarkpoolInstance as ExecutorInstance;
use std::{str::FromStr, time::Duration};
mod contract_interaction;
pub mod errors;
use crate::cli::Cli;
use renegade_util::err_str;

// -------------
// | Constants |
// -------------

/// Block polling interval
const BLOCK_POLLING_INTERVAL: Duration = Duration::from_millis(500);

/// Gas price multiplier to prevent transaction reverts by paying above basefee
const GAS_PRICE_MULTIPLIER: u64 = 2;

/// A type alias for the RPC client, which is an alloy middleware stack that
/// includes a signer derived from a raw private key, and a provider that
/// connects to the RPC endpoint over HTTP.
pub type ExecutorProvider = DynProvider;
/// A executor call builder type
pub type ExecutorCallBuilder<'a, C> = CallBuilder<&'a DynProvider, C>;

/// A configuration struct for the executor client, consists of relevant
/// contract addresses, and endpoint for setting up an RPC client, and a private
/// key for signing transactions.
pub struct ExecutorConfig {
    /// The address of the executor proxy contract.
    ///
    /// This is the main entrypoint to interaction with the executor.
    pub contract_address: String,
    /// HTTP-addressable RPC endpoint for the client to connect to
    pub rpc_url: String,
    /// The private key of the account to use for signing transactions
    pub private_key: PrivateKeySigner,
}

// ----------
// | Client |
// ----------

impl ExecutorConfig {
    /// Creates a new configuration
    pub fn new(contract_address: String, rpc_url: String, private_key: PrivateKeySigner) -> Self {
        Self { contract_address, rpc_url, private_key }
    }

    /// Constructs RPC clients capable of signing transactions from the
    /// configuration
    fn get_provider(&self) -> Result<ExecutorProvider, ExecutorConfigError> {
        let url = Url::parse(&self.rpc_url)
            .map_err(err_str!(ExecutorConfigError::RpcClientInitialization))?;

        let key = self.private_key.clone();
        let provider = ProviderBuilder::new().wallet(key).connect_http(url);

        // Set polling interval optimized for Base's fast block times
        provider.client().set_poll_interval(BLOCK_POLLING_INTERVAL);

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
}

impl ExecutorClient {
    /// Get a reference to the underlying provider
    pub fn provider(&self) -> &ExecutorProvider {
        self.contract.provider()
    }

    /// Creates a new ExecutorClient from CLI configuration
    pub fn new(cli: &Cli) -> Result<Self, ExecutorConfigError> {
        // Parse the private key
        let private_key = PrivateKeySigner::from_str(&cli.private_key)
            .map_err(err_str!(ExecutorConfigError::AddressParsing))?;

        // Create the configuration
        let config =
            ExecutorConfig::new(cli.contract_address.clone(), cli.rpc_url.clone(), private_key);

        // Create provider and contract instance
        let provider = config.get_provider()?;
        let contract_address = config.get_contract_address()?;
        let contract = ExecutorInstance::new(contract_address, provider);

        Ok(Self { contract })
    }
}

// ----------------
// | Transactions |
// ----------------

impl ExecutorClient {
    /// Send a txn and return the receipt
    async fn send_tx<C>(
        &self,
        tx: ExecutorCallBuilder<'_, C>,
        priority_fee_wei: U256,
    ) -> Result<TransactionReceipt, ExecutorError>
    where
        C: CallDecoder + Send + Sync,
    {
        let gas_price = self.get_adjusted_gas_price(priority_fee_wei).await?;
        let receipt = tx
            .gas_price(gas_price)
            .send()
            .await
            .map_err(ExecutorError::contract_interaction)?
            .get_receipt()
            .await
            .map_err(ExecutorError::contract_interaction)?;

        // Check for failure
        if !receipt.status() {
            let error_msg = format!("tx ({:#x}) failed with status 0", receipt.transaction_hash);
            return Err(ExecutorError::contract_interaction(error_msg));
        }

        Ok(receipt)
    }

    /// Computes a `gasPrice` from a caller-supplied priority fee and the base
    /// fee from the latest block.
    async fn get_adjusted_gas_price(&self, priority_fee_wei: U256) -> Result<u128, ExecutorError> {
        // Fetch the latest block to obtain the current base fee.
        let base_fee_per_gas = self.get_latest_block_base_fee().await?;

        // Apply our safety buffer: baseFee × GAS_PRICE_MULTIPLIER.
        let base_fee_buffer = base_fee_per_gas
            .checked_mul(U256::from(GAS_PRICE_MULTIPLIER))
            .ok_or_else(|| ExecutorError::contract_interaction("base fee overflow"))?;

        // Final gas price for a legacy tx: baseFee + buffer + priority fee.
        let gas_price_u256 = base_fee_buffer + priority_fee_wei;

        Ok(u256_to_u128(gas_price_u256)?)
    }

    /// Returns the base fee per gas for the latest block as a U256
    async fn get_latest_block_base_fee(&self) -> Result<U256, ExecutorError> {
        let latest_block = self
            .provider()
            .get_block(BlockId::latest())
            .await
            .map_err(ExecutorError::rpc)?
            .ok_or_else(|| ExecutorError::rpc("No latest block found"))?;

        let base_fee_per_gas_u128 = latest_block
            .header
            .base_fee_per_gas
            .ok_or_else(|| ExecutorError::rpc("No base fee in latest block"))?;
        let base_fee_per_gas = U256::from(base_fee_per_gas_u128);

        Ok(base_fee_per_gas)
    }
}
