//! Bebop API type definitions

#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use serde::{Deserialize, Serialize};

/// The subset of Bebop quote request query parameters that we support.
///
/// See: <https://api.bebop.xyz/router/ethereum/docs#/v1/get_quote_v1_quote_get>
#[derive(Serialize, Deserialize)]
pub struct BebopQuoteParams {
    /// The tokens that will be supplied by the taker.
    ///
    /// This is a comma-separated list of token addresses.
    pub sell_tokens: String,
    /// The tokens that will be supplied by the maker.
    ///
    /// This is a comma-separated list of token addresses.
    pub buy_tokens: String,
    /// The amount of each taker token, order respective to sell_tokens.
    ///
    /// This is a comma-separated list of amounts in atoms.
    pub sell_amounts: String,
    /// Address which will sign the order
    pub taker_address: String,
    /// The token approval type to use for the quoted order.
    pub approval_type: ApprovalType,
    /// Whether the solver should execute the order & fold gas costs
    /// into the price.
    ///
    /// Set to `false` to self-execute.
    pub gasless: bool,
    /// The slippage tolerance to use.
    pub slippage: f64,
}

/// The type of approval to use for the quoted order.
///
/// We currently only support standard ERC20 approval.
#[derive(Serialize, Deserialize)]
pub enum ApprovalType {
    Standard,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopSuccessfulQuoteResponse {
    routes: Vec<BebopRoute>,
    errors: BebopQuoteError,
    best_price: BebopRouteSource,
}

#[derive(Serialize, Deserialize)]
pub struct BebopRoute {
    #[serde(rename = "type")]
    pub route_type: BebopRouteSource,
    quote: BebopQuote,
}

#[derive(Serialize, Deserialize)]
pub enum BebopRouteSource {
    #[serde(rename = "JAMv2")]
    JAM,
    #[serde(rename = "PMMv3")]
    PMM,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum BebopQuote {
    JAM(BebopJamQuote),
    PMM(BebopPmmQuote),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopJamQuote {
    #[serde(flatten)]
    quote: BebopQuoteInfo,
    hooks_hash: String,
    to_sign: BebopSignableJamOrder,
    solver: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopPmmQuote {
    #[serde(flatten)]
    quote: BebopQuoteInfo,
    makers: Vec<String>,
    to_sign: BebopSignablePmmOrder,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopQuoteInfo {
    slippage: f64,
    gas_fee: BebopGasFee,
    buy_tokens: BebopBuyToken,
    sell_tokens: BebopQuotedTokenInfo,
    settlement_address: String,
    approval_target: String,
    required_signatures: Vec<String>,
    price_impact: Option<f64>,
    tx: Option<BebopTxData>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopGasFee {
    native: String,
    usd: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopBuyToken {
    #[serde(flatten)]
    info: BebopQuotedTokenInfo,
    minimum_amount: String,
    amount_before_fee: Option<String>,
    delta_from_expected: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopQuotedTokenInfo {
    amount: String,
    decimals: u8,
    price_usd: Option<f64>,
    symbol: String,
    price: Option<f64>,
    price_before_fee: Option<f64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopTxData {
    from: Option<String>,
    to: String,
    value: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopSignableJamOrder {
    taker: String,
    receiver: String,
    expiry: u128,
    exclusivity_deadline: u128,
    nonce: String,
    executor: String,
    partner_info: String,
    sell_tokens: Vec<String>,
    buy_tokens: Vec<String>,
    sell_amounts: Vec<String>,
    buy_amounts: Vec<String>,
    hooks_hash: String,
}

#[derive(Serialize, Deserialize)]
pub struct BebopSignablePmmOrder {
    partner_id: u128,
    expiry: u128,
    taker_address: String,
    maker_address: String,
    maker_nonce: String,
    taker_token: String,
    maker_token: String,
    taker_amount: String,
    maker_amount: String,
    receiver: String,
    packed_commands: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopQuoteError {
    error_code: u128,
    message: Option<String>,
    fee: Option<BebopMinSizeFee>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BebopMinSizeFee {
    ether: f64,
    usd: f64,
}
