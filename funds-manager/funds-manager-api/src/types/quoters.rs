//! API types for quoter management
use std::fmt::Display;

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

/// An enum used to specify supported execution venues
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SupportedExecutionVenue {
    /// The Lifi venue
    Lifi,
    /// The Cowswap venue
    Cowswap,
    /// The Bebop venue
    Bebop,
    /// The Okx venue
    Okx,
}

impl Display for SupportedExecutionVenue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedExecutionVenue::Lifi => write!(f, "Lifi"),
            SupportedExecutionVenue::Cowswap => write!(f, "Cowswap"),
            SupportedExecutionVenue::Bebop => write!(f, "Bebop"),
            SupportedExecutionVenue::Okx => write!(f, "Okx"),
        }
    }
}

/// Parameters for requesting a quote to be executed
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteParams {
    /// The token that should be transferred. Can be the address or the symbol
    pub from_token: String,
    /// The token that should be transferred to. Can be the address or the
    /// symbol
    pub to_token: String,
    /// The amount that should be sent including all decimals (e.g. 1000000 for
    /// 1 USDC (6 decimals))
    #[serde(with = "u256_string_serialization")]
    pub from_amount: U256,
    /// The slippage tolerance for the quote, as a decimal (e.g. 0.0001 for
    /// 1 basis point, or 0.01%)
    ///
    /// If not provided, the default slippage tolerance will be used.
    pub slippage_tolerance: Option<f64>,
    /// Whether to increase the price deviation tolerance when it is exceeded
    /// in fetched quotes
    #[serde(default)]
    pub increase_price_deviation: bool,
    /// The venue to use for the quote. If not provided, the best quote across
    /// all venues will be selected.
    pub venue: Option<SupportedExecutionVenue>,
}

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
    /// The chain ID that the quote was generated on
    pub chain_id: u64,
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
    pub quote_params: QuoteParams,
    /// The tokens to exclude from the swaps
    #[serde(default)]
    pub exclude_tokens: Vec<String>,
}

/// The request body for withdrawing USDC to Hyperliquid from the quoter hot
/// wallet
#[derive(Debug, Serialize, Deserialize)]
pub struct WithdrawToHyperliquidRequest {
    /// The amount of USDC to withdraw, in decimal format (i.e., whole units)
    pub amount: f64,
}
