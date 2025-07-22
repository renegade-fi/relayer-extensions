//! API types for quoter management
use alloy_primitives::{hex, Address, Bytes, U256};
use renegade_common::types::{
    chain::Chain,
    token::{Token, USDC_TICKER},
};
use serde::{Deserialize, Serialize};

use crate::{
    serialization::{f64_string_serialization, u256_string_serialization},
    u256_try_into_u128,
};

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

/// The subset of the quote response forwarded to consumers of this client
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionQuote {
    /// The token address we're buying
    pub buy_token_address: Address,
    /// The token address we're selling
    pub sell_token_address: Address,
    /// The amount of tokens to sell
    #[serde(with = "u256_string_serialization")]
    pub sell_amount: U256,
    /// The amount of tokens expected to be received
    #[serde(with = "u256_string_serialization")]
    pub buy_amount: U256,
    /// The minimum amount of tokens expected to be received
    #[serde(with = "u256_string_serialization")]
    pub buy_amount_min: U256,
    /// The submitting address
    pub from: Address,
    /// The swap contract address
    pub to: Address,
    /// The calldata for the swap
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
    /// The gas limit for the swap
    #[serde(with = "u256_string_serialization")]
    pub gas_limit: U256,
}

/// An execution quote, augmented with additional contextual data
#[derive(Clone, Debug)]
pub struct AugmentedExecutionQuote {
    /// The quote
    pub quote: ExecutionQuote,
    /// The chain the quote is for
    pub chain: Chain,
}

impl AugmentedExecutionQuote {
    /// Create a new augmented execution quote
    pub fn new(quote: ExecutionQuote, chain: Chain) -> Self {
        Self { quote, chain }
    }

    /// Convert the quote to a canonical string representation for HMAC signing
    pub fn to_canonical_string(&self) -> String {
        format!(
            "{}{}{}{}{}{}{}{}{}{}{}{}",
            self.quote.buy_token_address,
            self.quote.sell_token_address,
            self.quote.sell_amount,
            self.quote.buy_amount,
            self.quote.from,
            self.quote.to,
            hex::encode(&self.quote.data),
            self.quote.value,
            self.quote.gas_price,
            self.quote.estimated_gas,
            self.quote.gas_limit,
            self.chain,
        )
    }

    /// Get the buy token address as a formatted string
    pub fn get_buy_token_address(&self) -> String {
        format!("{:#x}", self.quote.buy_token_address)
    }

    /// Get the sell token address as a formatted string
    pub fn get_sell_token_address(&self) -> String {
        format!("{:#x}", self.quote.sell_token_address)
    }

    /// Get the from address as a formatted string
    pub fn get_from_address(&self) -> String {
        format!("{:#x}", self.quote.from)
    }

    /// Get the to address as a formatted string
    pub fn get_to_address(&self) -> String {
        format!("{:#x}", self.quote.to)
    }

    /// Get the buy amount as a decimal-corrected string
    pub fn get_decimal_corrected_buy_amount(&self) -> Result<f64, String> {
        let buy_amount = u256_try_into_u128(self.quote.buy_amount)?;
        Ok(self.get_buy_token().convert_to_decimal(buy_amount))
    }

    /// Get the sell amount as a decimal-corrected string
    pub fn get_decimal_corrected_sell_amount(&self) -> Result<f64, String> {
        let sell_amount = u256_try_into_u128(self.quote.sell_amount)?;
        Ok(self.get_sell_token().convert_to_decimal(sell_amount))
    }

    /// Get the buy amount min as a decimal-corrected string
    pub fn get_decimal_corrected_buy_amount_min(&self) -> Result<f64, String> {
        let buy_amount_min = u256_try_into_u128(self.quote.buy_amount_min)?;
        Ok(self.get_buy_token().convert_to_decimal(buy_amount_min))
    }
}

impl AugmentedExecutionQuote {
    /// Get the price in units of USDC per base token.
    /// If a custom buy amount is provided, it is used in place of the standard
    /// buy amount.
    pub fn get_price(&self, buy_amount: Option<U256>) -> Result<f64, String> {
        let buy_amount = u256_try_into_u128(buy_amount.unwrap_or(self.quote.buy_amount))?;
        let decimal_buy_amount = self.get_buy_token().convert_to_decimal(buy_amount);

        let decimal_sell_amount = self.get_decimal_corrected_sell_amount()?;

        let buy_per_sell = decimal_buy_amount / decimal_sell_amount;
        if self.is_buy() {
            Ok(1.0 / buy_per_sell)
        } else {
            Ok(buy_per_sell)
        }
    }

    /// Returns the non-USDC token
    pub fn get_base_token(&self) -> Token {
        if self.is_buy() {
            self.get_buy_token()
        } else {
            self.get_sell_token()
        }
    }

    /// Return true if the sell token is USDC
    pub fn is_buy(&self) -> bool {
        let usdc_mint = &Token::from_ticker_on_chain(USDC_TICKER, self.chain).get_alloy_address();
        &self.quote.sell_token_address == usdc_mint
    }

    /// Returns the token being bought
    pub fn get_buy_token(&self) -> Token {
        Token::from_addr_on_chain(&self.get_buy_token_address(), self.chain)
    }

    /// Returns the token being sold
    pub fn get_sell_token(&self) -> Token {
        Token::from_addr_on_chain(&self.get_sell_token_address(), self.chain)
    }

    /// Returns the volume in USDC
    pub fn get_quote_amount(&self) -> Result<f64, String> {
        if self.is_buy() {
            self.get_decimal_corrected_sell_amount()
        } else {
            self.get_decimal_corrected_buy_amount()
        }
    }

    /// Returns the notional volume in USDC, taking into account the actual
    /// buy amount for sell orders
    pub fn notional_volume_usdc(&self, buy_amount_actual: U256) -> Result<f64, String> {
        if self.is_buy() {
            self.get_decimal_corrected_sell_amount()
        } else {
            let buy_amount = u256_try_into_u128(buy_amount_actual)?;

            Ok(self.get_buy_token().convert_to_decimal(buy_amount))
        }
    }
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
    pub quote: ExecutionQuote,
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
