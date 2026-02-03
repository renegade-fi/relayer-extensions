//! Logic for calculating refund info for a sponsored match

use alloy_primitives::U256;
use auth_server_api::GasSponsorshipInfo;
use bigdecimal::{BigDecimal, FromPrimitive};
use renegade_constants::NATIVE_ASSET_ADDRESS;
use renegade_external_api::types::{
    ApiExternalQuote, BoundedExternalMatchApiBundle, ExternalOrder,
};
use renegade_types_core::Token;
use renegade_util::hex::address_to_hex_string;
use tracing::info;

use super::{CachedSponsorshipInfo, WETH_TICKER};
use crate::{error::AuthServerError, server::Server};

// -------------
// | Constants |
// -------------

/// The number of Wei in 1 ETH, as an `U256`.
/// Concretely, this is 10^18
const ALLOY_WEI_IN_ETHER: U256 = U256::from_limbs([1_000_000_000_000_000_000_u64, 0, 0, 0]);

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
    ) -> Result<U256, AuthServerError> {
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
    ) -> Result<U256, AuthServerError> {
        let buy_mint = address_to_hex_string(&order.output_mint);
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

        let conversion_rate_u256 =
            U256::try_from(conversion_rate_bigint).map_err(AuthServerError::gas_sponsorship)?;

        Ok(conversion_rate_u256)
    }
}

// -----------
// | Helpers |
// -----------

/// Revert the effect of gas sponsorship from the given quote
///
/// The `cached_info` contains both the gas sponsorship info and the original
/// price from the relayer's signed quote, which must be restored exactly for
/// signature verification.
pub fn remove_gas_sponsorship_from_quote(
    quote: &mut ApiExternalQuote,
    cached_info: &CachedSponsorshipInfo,
) -> Result<(), AuthServerError> {
    let gas_sponsorship_info = &cached_info.gas_sponsorship_info;

    quote.match_result.output_amount -= gas_sponsorship_info.refund_amount;
    quote.receive.amount -= gas_sponsorship_info.refund_amount;
    quote.price.price = cached_info.original_price;

    // Subtract the refund amount from the exact output amount requested in the
    // order, to match the order received & signed by the relayer
    if requires_exact_output_amount_update(&quote.order, gas_sponsorship_info) {
        apply_gas_sponsorship_to_exact_output_amount(&mut quote.order, gas_sponsorship_info)?;
    }

    Ok(())
}

/// Update a quote to reflect a gas sponsorship refund.
/// This method assumes that the refund was in-kind, i.e. that the refund
/// amount is in terms of the buy-side token.
///
/// Returns the original price from the quote before modification, which
/// must be cached and restored during assembly to ensure signature
/// verification succeeds.
pub fn apply_gas_sponsorship_to_quote(
    quote: &mut ApiExternalQuote,
    gas_sponsorship_info: &GasSponsorshipInfo,
) -> Result<f64, AuthServerError> {
    info!("Updating quote to reflect gas sponsorship");

    // Capture the original price before modification
    let original_price = quote.price.price;

    quote.match_result.output_amount += gas_sponsorship_info.refund_amount;

    let input_amt_f64 = quote.match_result.input_amount as f64;
    let output_amt_f64 = quote.match_result.output_amount as f64;
    let price = output_amt_f64 / input_amt_f64;

    quote.price.price = price;
    quote.receive.amount += gas_sponsorship_info.refund_amount;

    // Update order to match what was requested by the user
    if requires_exact_output_amount_update(&quote.order, gas_sponsorship_info) {
        remove_gas_sponsorship_from_exact_output_amount(&mut quote.order, gas_sponsorship_info)?;
    }

    Ok(original_price)
}

/// Update a match bundle to reflect a gas sponsorship refund.
/// This method assumes that the refund was in-kind, i.e. that the refund
/// amount is in terms of the buy-side token.
pub(crate) fn apply_gas_sponsorship_to_match_bundle(
    match_bundle: &mut BoundedExternalMatchApiBundle,
    refund_amount: u128,
) {
    info!("Updating match bundle to reflect gas sponsorship");
    match_bundle.max_receive.amount += refund_amount;
    match_bundle.min_receive.amount += refund_amount;
}

/// Check if the exact output amount requested in the order should be updated
/// to reflect the refund amount
pub fn requires_exact_output_amount_update(
    order: &ExternalOrder,
    gas_sponsorship_info: &GasSponsorshipInfo,
) -> bool {
    order.use_exact_output_amount && gas_sponsorship_info.requires_match_result_update()
}

/// Account for the given gas sponsorship refund in the exact output amount
/// requested in the order. Concretely, this means subtracting the refund amount
/// from the exact output amount, so that the order matched by the relayer bears
/// the desired output amount only _after_ the refund is issued.
pub fn apply_gas_sponsorship_to_exact_output_amount(
    order: &mut ExternalOrder,
    gas_sponsorship_info: &GasSponsorshipInfo,
) -> Result<(), AuthServerError> {
    if !order.use_exact_output_amount {
        return Err(AuthServerError::custom("order does not use exact output amount"));
    }

    order.output_amount -= gas_sponsorship_info.refund_amount;

    Ok(())
}

/// Remove the effects of gas sponsorship from the exact output amount requested
/// in the order. The order passed in is assumed to have already had the gas
/// sponsorship refund subtracted from the exact output amount. This function
/// reverses that operation, so that the order is restored to its original
/// state.
pub fn remove_gas_sponsorship_from_exact_output_amount(
    order: &mut ExternalOrder,
    gas_sponsorship_info: &GasSponsorshipInfo,
) -> Result<(), AuthServerError> {
    if !order.use_exact_output_amount {
        return Err(AuthServerError::custom("order does not use exact output amount"));
    }

    order.output_amount += gas_sponsorship_info.refund_amount;

    Ok(())
}
