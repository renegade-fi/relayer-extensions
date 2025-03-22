//! API types for quoter management
use ethers::types::{Address, Bytes, U256};
use serde::{Deserialize, Serialize};

use crate::serialization::{
    address_string_serialization, bytes_string_serialization, u256_string_serialization,
};

// --------------
// | Api Routes |
// --------------

/// The route to retrieve the address to deposit custody funds to
pub const GET_DEPOSIT_ADDRESS_ROUTE: &str = "deposit-address";
/// The route to withdraw funds from custody
pub const WITHDRAW_CUSTODY_ROUTE: &str = "withdraw";
/// The route to fetch an execution quote on the quoter hot wallet
///
/// Expected query parameters (proxied directly to LiFi API):
/// - fromChain: Source chain ID
/// - toChain: Destination chain ID
/// - fromToken: Source token address
/// - toToken: Destination token address
/// - fromAddress: Source wallet address
/// - toAddress: Destination wallet address
/// - fromAmount: Source token amount
/// - order: Order preference for routing (e.g. 'CHEAPEST')
/// - slippage: Slippage tolerance as a decimal (e.g. 0.0001 for 0.01%)
pub const GET_EXECUTION_QUOTE_ROUTE: &str = "get-execution-quote";
/// The route to execute a swap on the quoter hot wallet
pub const EXECUTE_SWAP_ROUTE: &str = "execute-swap";

// -------------
// | Api Types |
// -------------

/// A response containing the deposit address
#[derive(Debug, Serialize, Deserialize)]
pub struct DepositAddressResponse {
    /// The deposit address
    pub address: String,
}

/// The request body for withdrawing funds from custody
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WithdrawFundsRequest {
    /// The mint of the asset to withdraw
    pub mint: String,
    /// The amount of funds to withdraw
    pub amount: f64,
    /// The address to withdraw to
    pub address: String,
}

// --- Execution --- //

/// The subset of the quote response forwarded to consumers of this client
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionQuote {
    /// The token address we're buying
    #[serde(with = "address_string_serialization")]
    pub buy_token_address: Address,
    /// The token address we're selling
    #[serde(with = "address_string_serialization")]
    pub sell_token_address: Address,
    /// The amount of tokens to sell
    #[serde(with = "u256_string_serialization")]
    pub sell_amount: U256,
    /// The submitting address
    #[serde(with = "address_string_serialization")]
    pub from: Address,
    /// The 0x swap contract address
    #[serde(with = "address_string_serialization")]
    pub to: Address,
    /// The calldata for the swap
    #[serde(with = "bytes_string_serialization")]
    pub data: Bytes,
    /// The value of the tx; should be zero
    #[serde(with = "u256_string_serialization")]
    pub value: U256,
    /// The gas price used in the swap
    #[serde(with = "u256_string_serialization")]
    pub gas_price: U256,
    /// The estimated gas for the swap
    #[serde(with = "u256_string_serialization")]
    pub estimated_gas: U256,
}

/// The request body for fetching a quote from the execution venue
#[derive(Debug, Serialize, Deserialize)]
pub struct GetExecutionQuoteResponse {
    /// The quote, directly from the execution venue
    pub quote: serde_json::Value,
}

/// The request body for executing a swap on the execution venue
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecuteSwapRequest {
    /// The quote, implicitly accepted by the caller by its presence in this
    /// request
    pub quote: ExecutionQuote,
}

/// The response body for executing a swap on the execution venue
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecuteSwapResponse {
    /// The tx hash of the swap
    pub tx_hash: String,
}
