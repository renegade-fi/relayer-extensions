//! Cowswap API type definitions.
//! See: <https://docs.cow.fi/cow-protocol/reference/apis/orderbook>

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

use funds_manager_api::serialization::u256_string_serialization;

/// The kind of order to request a Cowswap quote for.
///
/// We only support sell orders, to maintain the convention
/// of specifying a sell amount in the quote request.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum OrderKind {
    /// A sell order
    Sell,
}

/// The scheme used to sign an order.
///
/// We only support EIP-712 signatures.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SigningScheme {
    /// EIP-712 signatures
    Eip712,
}

/// The subset of Cowswap quote request parameters that we support
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderQuoteRequest {
    /// The address of the token being sold, as a hex string
    sell_token: String,
    /// The address of the token being bought, as a hex string
    buy_token: String,
    /// The sending wallet address, as a hex string
    from: String,
    /// The kind of order (buy/sell) to request a quote for.
    ///
    /// In our case, this should *always* be a sell order, such that
    /// we specify a sell amount as opposed to a buy amount.
    kind: OrderKind,
    /// The amount of the token being sold.
    ///
    /// Fees are deducted from this value.
    #[serde(with = "u256_string_serialization")]
    sell_amount_before_fee: U256,
    // TODO: Add `app_data` and `app_data_hash` fields,
    // which will be used to specify slippage tolerance.
}

/// The parameters of an order that was quoted
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderParameters {
    /// The address of the token being sold, as a hex string
    sell_token: String,
    /// The address of the token being bought, as a hex string
    buy_token: String,
    /// The amount of the token being sold
    #[serde(with = "u256_string_serialization")]
    sell_amount: U256,
    /// The amount of the token being bought
    #[serde(with = "u256_string_serialization")]
    buy_amount: U256,
    /// The Unix timestamp until which the order is valid
    valid_to: u32,
    /// Amount of sell token (in atoms) used to cover network fees.
    ///
    /// Needs to be zero (and incorporated into the limit price) when placing
    /// the order.
    #[serde(with = "u256_string_serialization")]
    fee_amount: U256,
    /// The kind of quote requested.
    kind: OrderKind,
    /// Whether the order is partially fillable (otherwise, fill-or-kill)
    partially_fillable: bool,
}

/// The parameters of an order that was quoted,
/// augmented with the *hash* of the `app_data` that was used to request the
/// quote.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderParametersWithAppData {
    /// The parameters of the order
    #[serde(flatten)]
    order: OrderParameters,
    /// Hex-encoded keccak-256 hash of the `app_data` that the quote
    /// was requested with.
    ///
    /// NOTE: Until we implement specifying `app_data` ourselves, we expect this
    /// to be
    /// `0xb48d38f93eaa084033fc5970bf96e559c33c4cdc07d889ab00b4d63f9590739d`,
    /// the keccak-256 hash of `"{}"`.
    app_data: String,
}

/// The response to a Cowswap quote request
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderQuoteResponse {
    /// The parameters of the order that was quoted
    quote: OrderParametersWithAppData,
    /// ISO-8601 string represeneting the expiration date of the offered fee.
    expiration: String,
    /// The ID of the quote
    id: u64,
    /// Whether the quoted amounts were simulated
    verified: bool,
}

/// Data required to create an order on Cowswap
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OrderCreation {
    /// The parameters of the order to create
    #[serde(flatten)]
    order: OrderParameters,
    /// The signature of the order
    signing_scheme: SigningScheme,
    /// The EIP-712 signature over the order.
    ///
    /// Concretely, the hex-encoded `r || s || v` values, totaling 65 bytes.
    // TODO: Determine if `v` is expected to be 0/1 or 27/28.
    signature: String,
    /// A string encoding of the JSON `app_data` that was used to request the
    /// quote.
    ///
    /// The UTF-8 encoding of this string must be the preimage of the `app_data`
    /// hash in the quote response.
    ///
    /// NOTE: Until we implement specifying `app_data` ourselves, we will set
    /// this to "{}".
    app_data: String,
}

/// A trade that was executed on Cowswap
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Trade {
    /// The block number at which the trade was executed
    block_number: u64,
    /// The index within the block at which the transaction containing the trade
    /// was included
    log_index: u64,
    /// UID of the order matched by this trade, as a hex string
    order_uid: String,
    /// The address of the trader, as a hex string
    owner: String,
    /// The address of the token being sold, as a hex string
    sell_token: String,
    /// The address of the token being bought, as a hex string
    buy_token: String,
    /// The amount sold in this trade, including fees
    #[serde(with = "u256_string_serialization")]
    sell_amount: U256,
    /// The amount sold in this trade, without fees
    #[serde(with = "u256_string_serialization")]
    sell_amount_before_fees: U256,
    /// The amount bought in this trade
    #[serde(with = "u256_string_serialization")]
    buy_amount: U256,
    /// The hash of the transaction in which this trade was settled
    tx_hash: String,
}
