//! Cowswap API type definitions.
//! See: <https://docs.cow.fi/cow-protocol/reference/apis/orderbook>

use std::{
    fmt::Display,
    time::{SystemTime, UNIX_EPOCH},
};

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::U256;
use renegade_common::types::{chain::Chain, token::Token};
use serde::{Deserialize, Serialize};

use funds_manager_api::serialization::u256_string_serialization;

use crate::execution_client::swap::DEFAULT_SLIPPAGE_TOLERANCE;

// -------------
// | Constants |
// -------------

/// The number of seconds for which an order is valid
const COWSWAP_ORDER_VALID_FOR: u32 = 2 * 60; // 2 minutes

/// The number of basis points in 1 unit
const BPS_PER_UNIT: f64 = 10_000.0;

/// The default `app_data` for an order.
const DEFAULT_APP_DATA: &str = "{}";

/// The kind of order to request a Cowswap quote for.
///
/// We only support sell orders, to maintain the convention
/// of specifying a sell amount in the quote request.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum OrderKind {
    /// A sell order
    Sell,
}

impl Display for OrderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrderKind::Sell => write!(f, "sell"),
        }
    }
}

/// The scheme used to sign an order.
///
/// We only support EIP-712 signatures.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum SigningScheme {
    /// EIP-712 signatures
    Eip712,
}

/// The subset of Cowswap quote request parameters that we support
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderQuoteRequest {
    /// The address of the token being sold, as a hex string
    pub sell_token: String,
    /// The address of the token being bought, as a hex string
    pub buy_token: String,
    /// The sending wallet address, as a hex string
    pub from: String,
    /// The kind of order (buy/sell) to request a quote for.
    ///
    /// In our case, this should *always* be a sell order, such that
    /// we specify a sell amount as opposed to a buy amount.
    pub kind: OrderKind,
    /// The amount of the token being sold.
    ///
    /// Fees are deducted from this value.
    #[serde(with = "u256_string_serialization")]
    pub sell_amount_before_fee: U256,
}

/// The parameters of an order that was quoted
#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderParameters {
    /// The address of the token being sold, as a hex string
    pub sell_token: String,
    /// The address of the token being bought, as a hex string
    pub buy_token: String,
    /// The amount of the token being sold
    #[serde(with = "u256_string_serialization")]
    pub sell_amount: U256,
    /// The amount of the token being bought
    #[serde(with = "u256_string_serialization")]
    pub buy_amount: U256,
    /// The Unix timestamp until which the order is valid
    pub valid_to: u32,
    /// Amount of sell token (in atoms) used to cover network fees.
    ///
    /// Needs to be zero (and incorporated into the limit price) when placing
    /// the order.
    #[serde(with = "u256_string_serialization")]
    pub fee_amount: U256,
    /// The kind of quote requested.
    pub kind: OrderKind,
    /// Whether the order is partially fillable (otherwise, fill-or-kill)
    pub partially_fillable: bool,
}

/// The response to a Cowswap quote request
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct OrderQuoteResponse {
    /// The parameters of the order that was quoted
    pub quote: OrderParameters,
    /// ISO-8601 string represeneting the expiration date of the offered fee.
    pub expiration: String,
    /// The ID of the quote
    pub id: u64,
    /// Whether the quoted amounts were simulated
    pub verified: bool,
}

impl OrderQuoteResponse {
    /// Get the token being sold
    pub fn get_sell_token(&self, chain: Chain) -> Token {
        Token::from_addr_on_chain(&self.quote.sell_token, chain)
    }

    /// Get the token being bought
    pub fn get_buy_token(&self, chain: Chain) -> Token {
        Token::from_addr_on_chain(&self.quote.buy_token, chain)
    }

    /// Get the amount of tokens being sold
    pub fn get_sell_amount(&self) -> U256 {
        self.quote.sell_amount
    }

    /// Get the amount of tokens being bought
    pub fn get_buy_amount(&self) -> U256 {
        self.quote.buy_amount
    }

    /// Get the sell/buy amounts after fees and slippage are deducted.
    /// Returns (sell_amount, buy_amount)
    ///
    /// Taken from implementation here:
    /// <https://github.com/cowprotocol/cow-sdk/blob/main/src/order-book/quoteAmountsAndCostsUtils.ts#L155>
    pub fn get_quote_amounts_after_costs(&self, slippage_tolerance: Option<f64>) -> (U256, U256) {
        let sell_amount_before_fees = self.get_sell_amount();
        let fee_amount = self.quote.fee_amount;

        let sell_amount_after_fees = sell_amount_before_fees + fee_amount;
        let buy_amount_after_fees = self.get_buy_amount();

        let slippage_tolerance_bps_f64 =
            slippage_tolerance.unwrap_or(DEFAULT_SLIPPAGE_TOLERANCE) * BPS_PER_UNIT;

        let slippage_tolerance_bps = U256::from(slippage_tolerance_bps_f64);

        // Currently, we only support sell orders, but we include this match statement
        // for type safety in the case that we support buy orders in the future.
        let (sell_amount_after_slippage, buy_amount_after_slippage) = match self.quote.kind {
            OrderKind::Sell => {
                let slippage_amount =
                    (buy_amount_after_fees * slippage_tolerance_bps) / U256::from(BPS_PER_UNIT);
                (sell_amount_after_fees, buy_amount_after_fees - slippage_amount)
            },
        };

        (sell_amount_after_slippage, buy_amount_after_slippage)
    }

    /// Whether the order is partially fillable.
    ///
    /// For now, we set this to `false`, to simplify polling for trade execution
    /// & swap cost accounting
    pub fn is_partially_fillable(&self) -> bool {
        false
    }

    /// Get the kind of order that was quoted
    pub fn get_order_kind(&self) -> OrderKind {
        self.quote.kind
    }

    /// Get the signing scheme that was used to sign the order.
    ///
    /// We currently only support EIP-712 signatures.
    pub fn get_signing_scheme(&self) -> SigningScheme {
        SigningScheme::Eip712
    }

    /// Get the JSON-stringified `app_data` for the quote
    pub fn get_app_data(&self) -> String {
        DEFAULT_APP_DATA.to_string()
    }

    /// Compute the Unix timestamp until which the order is valid
    pub fn compute_valid_to(&self) -> u32 {
        let now =
            SystemTime::now().duration_since(UNIX_EPOCH).expect("negative timestamp").as_secs()
                as u32;

        now + COWSWAP_ORDER_VALID_FOR
    }

    /// Sign the order represented by this quote with the given private key,
    /// using the EIP-712 signing scheme.
    // TODO: Implement EIP-712 signing, making sure to use `DEFAULT_APP_DATA_HASH`
    pub fn sign_order(&self, _private_key: &PrivateKeySigner) -> String {
        String::new()
    }
}

/// Data required to create an order on Cowswap
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderCreation {
    /// The parameters of the order to create
    #[serde(flatten)]
    pub order: OrderParameters,
    /// The signature of the order
    pub signing_scheme: SigningScheme,
    /// The EIP-712 signature over the order.
    ///
    /// Concretely, the hex-encoded `r || s || v` values, totaling 65 bytes.
    pub signature: String,
    /// A string encoding of the JSON `app_data` that was used to request the
    /// quote.
    ///
    /// The UTF-8 encoding of this string must be the preimage of the `app_data`
    /// hash in the quote response.
    ///
    /// In our case, this should always be "{}".
    pub app_data: String,
}

/// A trade that was executed on Cowswap
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    /// The block number at which the trade was executed
    pub block_number: u64,
    /// The index within the block at which the transaction containing the trade
    /// was included
    pub log_index: u64,
    /// UID of the order matched by this trade, as a hex string
    pub order_uid: String,
    /// The address of the trader, as a hex string
    pub owner: String,
    /// The address of the token being sold, as a hex string
    pub sell_token: String,
    /// The address of the token being bought, as a hex string
    pub buy_token: String,
    /// The amount sold in this trade, including fees
    #[serde(with = "u256_string_serialization")]
    pub sell_amount: U256,
    /// The amount sold in this trade, without fees
    #[serde(with = "u256_string_serialization")]
    pub sell_amount_before_fees: U256,
    /// The amount bought in this trade
    #[serde(with = "u256_string_serialization")]
    pub buy_amount: U256,
    /// The hash of the transaction in which this trade was settled
    pub tx_hash: String,
}
