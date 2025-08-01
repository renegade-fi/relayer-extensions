//! API types for quoter management
use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

use crate::serialization::{f64_string_serialization, u256_string_serialization};

// --------------
// | Api Routes |
// --------------

/// The route to retrieve the address to deposit custody funds to
pub const GET_DEPOSIT_ADDRESS_ROUTE: &str = "deposit-address";
/// The route to withdraw funds from custody
pub const WITHDRAW_CUSTODY_ROUTE: &str = "withdraw";
/// The route to withdraw USDC to Hyperliquid from the quoter hot wallet
pub const WITHDRAW_TO_HYPERLIQUID_ROUTE: &str = "withdraw-to-hyperliquid";
/// The route to swap immediately on the quoter hot wallet,
/// fetching a quote and executing it without first returning it to the client
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
pub const SWAP_IMMEDIATE_ROUTE: &str = "swap-immediate";
/// The route to execute swaps to cover a target amount of a given token.
///
/// Expects the same query parameters as SWAP_IMMEDIATE_ROUTE, except for
/// `fromToken` and `fromAmount`, as this will be calculated by the endpoint
/// for each swap.
pub const SWAP_INTO_TARGET_TOKEN_ROUTE: &str = "swap-into-target-token";

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

/// A simplified representation of an execution quote, suitable for API
/// responses
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiExecutionQuote {
    /// The address of the token being sold
    pub sell_token_address: String,
    /// The address of the token being bought
    pub buy_token_address: String,
    /// The amount of the token being sold
    #[serde(with = "u256_string_serialization")]
    pub sell_amount: U256,
    /// The amount of the token being bought
    #[serde(with = "u256_string_serialization")]
    pub buy_amount: U256,
    /// The venue that provided the quote
    pub venue: String,
}

/// The subset of LiFi quote request query parameters that we support
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiFiQuoteParams {
    /// The token that should be transferred. Can be the address or the symbol
    pub from_token: String,
    /// The token that should be transferred to. Can be the address or the
    /// symbol
    pub to_token: String,
    /// The amount that should be sent including all decimals (e.g. 1000000 for
    /// 1 USDC (6 decimals))
    #[serde(with = "u256_string_serialization")]
    pub from_amount: U256,
    /// The sending wallet address
    pub from_address: String,
    /// The receiving wallet address. If none is provided, the fromAddress will
    /// be used
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_address: Option<String>,
    /// The ID of the sending chain
    pub from_chain: usize,
    /// The ID of the receiving chain
    pub to_chain: usize,
    /// The maximum allowed slippage for the transaction as a decimal value.
    /// 0.005 represents 0.5%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage: Option<f64>,
    /// The maximum price impact for the transaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price_impact: Option<f64>,
    /// Timing setting to wait for a certain amount of swap rates. In the format
    /// minWaitTime-${minWaitTimeMs}-${startingExpectedResults}-${reduceEveryMs}.
    /// Please check docs.li.fi for more details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_step_timing_strategies: Option<Vec<String>>,
    /// Which kind of route should be preferred
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// Parameter to skip transaction simulation. The quote will be returned
    /// faster but the transaction gas limit won't be accurate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_simulation: Option<bool>,
    /// List of exchanges that are allowed for this transaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_exchanges: Option<Vec<String>>,
    /// List of exchanges that are not allowed for this transaction
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_exchanges: Option<Vec<String>>,
}

/// The response body for executing an immediate swap
#[derive(Debug, Serialize, Deserialize)]
pub struct SwapImmediateResponse {
    /// The quote that was executed
    pub quote: ApiExecutionQuote,
    /// The tx hash of the swap
    pub tx_hash: String,
    /// The execution cost in USD
    ///
    /// This is in whole USD as a floating point value, i.e. $10 will be
    /// represented as 10.0
    #[serde(with = "f64_string_serialization")]
    pub execution_cost: f64,
}

/// The request body for executing a swap to cover a target amount of a given
/// token
#[derive(Debug, Serialize, Deserialize)]
pub struct SwapIntoTargetTokenRequest {
    /// The target amount of the token to cover, in decimal format (i.e., whole
    /// units)
    pub target_amount: f64,
    /// The quote parameters for the swap. The `from_token` and `from_amount`
    /// fields will be ignored and calculated by the server, but they are still
    /// required to be set.
    pub quote_params: LiFiQuoteParams,
}

/// The request body for withdrawing USDC to Hyperliquid from the quoter hot
/// wallet
#[derive(Debug, Serialize, Deserialize)]
pub struct WithdrawToHyperliquidRequest {
    /// The amount of USDC to withdraw, in decimal format (i.e., whole units)
    pub amount: f64,
}
