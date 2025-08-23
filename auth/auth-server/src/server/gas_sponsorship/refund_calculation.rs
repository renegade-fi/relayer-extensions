//! Logic for calculating refund info for a sponsored match

use alloy_primitives::U256 as AlloyU256;
use auth_server_api::GasSponsorshipInfo;
use bigdecimal::{BigDecimal, FromPrimitive};
use renegade_api::http::external_match::{
    ApiExternalMatchResult, ApiExternalQuote, AtomicMatchApiBundle, ExternalOrder,
    MalleableAtomicMatchApiBundle,
};
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;
use renegade_constants::NATIVE_ASSET_ADDRESS;
use renegade_util::hex::biguint_to_hex_addr;
use tracing::info;

use crate::{error::AuthServerError, server::Server};

use super::WETH_TICKER;

// -------------
// | Constants |
// -------------

/// The number of Wei in 1 ETH, as an `AlloyU256`.
/// Concretely, this is 10^18
const ALLOY_WEI_IN_ETHER: AlloyU256 =
    AlloyU256::from_limbs([1_000_000_000_000_000_000_u64, 0, 0, 0]);

/// The error message emitted when converting an f64 price to a `BigDecimal`
/// fails
const ERR_PRICE_BIGDECIMAL_CONVERSION: &str = "failed to convert price to BigDecimal";

// ---------------
// | Server Impl |
// ---------------

impl Server {
    /// Get the amount to refund for a given match result
    pub async fn compute_refund_amount_for_order(
        &self,
        order: &ExternalOrder,
        refund_native_eth: bool,
    ) -> Result<AlloyU256, AuthServerError> {
        let conversion_rate =
            self.compute_conversion_rate_for_order(order, refund_native_eth).await?;

        let estimated_gas_cost = self.get_gas_cost_estimate().await;
        let refund_amount = (estimated_gas_cost * conversion_rate) / ALLOY_WEI_IN_ETHER;
        Ok(refund_amount)
    }

    /// Compute the conversion rate from ETH to the refund asset for the given
    /// order, in terms of nominal units of the refund asset per whole ETH.
    async fn compute_conversion_rate_for_order(
        &self,
        order: &ExternalOrder,
        refund_native_eth: bool,
    ) -> Result<AlloyU256, AuthServerError> {
        let buy_mint_biguint = match order.side {
            OrderSide::Buy => &order.base_mint,
            OrderSide::Sell => &order.quote_mint,
        };
        let buy_mint = biguint_to_hex_addr(buy_mint_biguint);
        let native_eth_buy = buy_mint == NATIVE_ASSET_ADDRESS.to_lowercase();

        let weth_addr = Token::from_ticker(WETH_TICKER).get_addr();
        let weth_buy = buy_mint == weth_addr;

        if refund_native_eth || native_eth_buy || weth_buy {
            // If we're deliberately refunding via native ETH, or the buy-side token
            // is native ETH or WETH, then the conversion rate is 1:1.
            // However, this method is expected to return the conversion rate in
            // terms of nominal units of the refund asset per whole ETH, so we return
            // the value of wei per Ether.
            return Ok(ALLOY_WEI_IN_ETHER);
        }

        let eth_price_f64 = self.price_reporter_client.get_eth_price().await?;
        let eth_price = BigDecimal::from_f64(eth_price_f64)
            .ok_or(AuthServerError::gas_sponsorship(ERR_PRICE_BIGDECIMAL_CONVERSION))?;

        let buy_token_price =
            self.price_reporter_client.get_price_usd(&buy_mint, self.chain).await?;

        let conversion_rate = eth_price / buy_token_price;

        // Convert the scaled rate to a U256. We can use the `BigInt` component of the
        // `BigDecimal` directly because we round to 0 digits after the decimal.
        let (conversion_rate_bigint, _) =
            conversion_rate.round(0 /* round_digits */).into_bigint_and_scale();

        let conversion_rate_u256 = AlloyU256::try_from(conversion_rate_bigint)
            .map_err(AuthServerError::gas_sponsorship)?;

        Ok(conversion_rate_u256)
    }
}

// -----------
// | Helpers |
// -----------

/// Revert the effect of gas sponsorship from the given quote
pub fn remove_gas_sponsorship_from_quote(
    quote: &mut ApiExternalQuote,
    gas_sponsorship_info: &GasSponsorshipInfo,
) {
    remove_gas_sponsorship_from_match_result(
        &mut quote.match_result,
        gas_sponsorship_info.refund_amount,
    );

    let base_amt_f64 = quote.match_result.base_amount as f64;
    let quote_amt_f64 = quote.match_result.quote_amount as f64;
    let price = quote_amt_f64 / base_amt_f64;

    quote.price.price = price.to_string();
    quote.receive.amount -= gas_sponsorship_info.refund_amount;

    // Subtract the refund amount from the exact output amount requested in the
    // order, to match the order received & signed by the relayer
    if requires_exact_output_amount_update(&quote.order, gas_sponsorship_info) {
        apply_gas_sponsorship_to_exact_output_amount(&mut quote.order, gas_sponsorship_info);
    }
}

/// Update a quote to reflect a gas sponsorship refund.
/// This method assumes that the refund was in-kind, i.e. that the refund
/// amount is in terms of the buy-side token.
pub fn apply_gas_sponsorship_to_quote(
    quote: &mut ApiExternalQuote,
    gas_sponsorship_info: &GasSponsorshipInfo,
) -> Result<(), AuthServerError> {
    info!("Updating quote to reflect gas sponsorship");

    apply_gas_sponsorship_to_match_result(
        &mut quote.match_result,
        gas_sponsorship_info.refund_amount,
    );

    let base_amt_f64 = quote.match_result.base_amount as f64;
    let quote_amt_f64 = quote.match_result.quote_amount as f64;
    let price = quote_amt_f64 / base_amt_f64;

    quote.price.price = price.to_string();
    quote.receive.amount += gas_sponsorship_info.refund_amount;

    // Update order to match what was requested by the user
    if requires_exact_output_amount_update(&quote.order, gas_sponsorship_info) {
        remove_gas_sponsorship_from_exact_output_amount(&mut quote.order, gas_sponsorship_info);
    }

    Ok(())
}

/// Update a match bundle to reflect a gas sponsorship refund.
/// This method assumes that the refund was in-kind, i.e. that the refund
/// amount is in terms of the buy-side token.
pub(crate) fn apply_gas_sponsorship_to_match_bundle(
    match_bundle: &mut AtomicMatchApiBundle,
    refund_amount: u128,
) {
    info!("Updating match bundle to reflect gas sponsorship");
    apply_gas_sponsorship_to_match_result(&mut match_bundle.match_result, refund_amount);
    match_bundle.receive.amount += refund_amount;
}

/// Update a match result to reflect a gas sponsorship refund.
/// This method assumes that the refund was in-kind, i.e. that the refund
/// amount is in terms of the buy-side token.
pub(crate) fn apply_gas_sponsorship_to_match_result(
    match_result: &mut ApiExternalMatchResult,
    refund_amount: u128,
) {
    let (base_amount, quote_amount) = match match_result.direction {
        OrderSide::Buy => (match_result.base_amount + refund_amount, match_result.quote_amount),
        OrderSide::Sell => (match_result.base_amount, match_result.quote_amount + refund_amount),
    };

    match_result.base_amount = base_amount;
    match_result.quote_amount = quote_amount;
}

/// Remove the effects of gas sponsorship from a match result.
/// This method assumes that the refund was in-kind, i.e. that the refund
/// amount is in terms of the buy-side token.
pub(crate) fn remove_gas_sponsorship_from_match_result(
    match_result: &mut ApiExternalMatchResult,
    refund_amount: u128,
) {
    let (base_amount, quote_amount) = match match_result.direction {
        OrderSide::Buy => (match_result.base_amount - refund_amount, match_result.quote_amount),
        OrderSide::Sell => (match_result.base_amount, match_result.quote_amount - refund_amount),
    };

    match_result.base_amount = base_amount;
    match_result.quote_amount = quote_amount;
}

/// Apply a gas sponsorship refund a malleable match bundle
pub(crate) fn apply_gas_sponsorship_to_malleable_match_bundle(
    match_bundle: &mut MalleableAtomicMatchApiBundle,
    refund_amount: u128,
) {
    info!("Updating malleable match bundle to reflect gas sponsorship");
    match_bundle.max_receive.amount += refund_amount;
    match_bundle.min_receive.amount += refund_amount;
}

/// Check if the exact output amount requested in the order should be updated
/// to reflect the refund amount
pub fn requires_exact_output_amount_update(
    order: &ExternalOrder,
    gas_sponsorship_info: &GasSponsorshipInfo,
) -> bool {
    let exact_out_requested = match order.side {
        OrderSide::Buy => order.exact_base_output != 0,
        OrderSide::Sell => order.exact_quote_output != 0,
    };

    exact_out_requested && gas_sponsorship_info.requires_match_result_update()
}

/// Account for the given gas sponsorship refund in the exact output amount
/// requested in the order. Concretely, this means subtracting the refund amount
/// from the exact output amount, so that the order matched by the relayer bears
/// the desired output amount only _after_ the refund is issued.
pub fn apply_gas_sponsorship_to_exact_output_amount(
    order: &mut ExternalOrder,
    gas_sponsorship_info: &GasSponsorshipInfo,
) {
    match order.side {
        OrderSide::Buy => {
            order.exact_base_output -= gas_sponsorship_info.refund_amount;
        },
        OrderSide::Sell => {
            order.exact_quote_output -= gas_sponsorship_info.refund_amount;
        },
    }
}

/// Remove the effects of gas sponsorship from the exact output amount requested
/// in the order. The order passed in is assumed to have already had the gas
/// sponsorship refund subtracted from the exact output amount. This function
/// reverses that operation, so that the order is restored to its original
/// state.
pub fn remove_gas_sponsorship_from_exact_output_amount(
    order: &mut ExternalOrder,
    gas_sponsorship_info: &GasSponsorshipInfo,
) {
    match order.side {
        OrderSide::Buy => {
            order.exact_base_output += gas_sponsorship_info.refund_amount;
        },
        OrderSide::Sell => {
            order.exact_quote_output += gas_sponsorship_info.refund_amount;
        },
    }
}
