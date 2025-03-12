//! Logic for calculating refund info for a sponsored match

use alloy_primitives::U256 as AlloyU256;
use bigdecimal::{BigDecimal, FromPrimitive};
use renegade_api::http::external_match::{ApiExternalMatchResult, ApiExternalQuote};
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;
use renegade_constants::NATIVE_ASSET_ADDRESS;

use crate::{
    error::AuthServerError,
    server::{
        helpers::{ethers_u256_to_alloy_u256, get_nominal_buy_token_price},
        Server,
    },
};

// -------------
// | Constants |
// -------------

/// The number of Wei in 1 ETH, as an `AlloyU256`.
/// Concretely, this is 10^18
const ALLOY_WEI_IN_ETHER: AlloyU256 =
    AlloyU256::from_limbs([1_000_000_000_000_000_000_u64, 0, 0, 0]);

// ---------------
// | Server Impl |
// ---------------

impl Server {
    /// Get the amount to refund for a given match result
    pub async fn get_refund_amount(
        &self,
        match_result: &ApiExternalMatchResult,
        refund_native_eth: bool,
    ) -> Result<AlloyU256, AuthServerError> {
        let conversion_rate =
            self.maybe_fetch_conversion_rate(match_result, refund_native_eth).await?;

        let estimated_gas_cost = ethers_u256_to_alloy_u256(self.get_gas_cost_estimate().await);

        let refund_amount = if let Some(conversion_rate) = conversion_rate {
            (estimated_gas_cost * conversion_rate) / ALLOY_WEI_IN_ETHER
        } else {
            estimated_gas_cost
        };

        Ok(refund_amount)
    }

    /// Fetch the conversion rate from ETH to the buy-side token in the trade
    /// from the price reporter, if necessary.
    /// The conversion rate is in terms of nominal units of TOKEN per whole ETH.
    #[allow(clippy::unused_async)]
    async fn maybe_fetch_conversion_rate(
        &self,
        match_result: &ApiExternalMatchResult,
        refund_native_eth: bool,
    ) -> Result<Option<AlloyU256>, AuthServerError> {
        let buy_mint = match match_result.direction {
            OrderSide::Buy => &match_result.base_mint,
            OrderSide::Sell => &match_result.quote_mint,
        };
        let native_eth_buy = buy_mint.to_lowercase() == NATIVE_ASSET_ADDRESS.to_lowercase();

        let weth_addr = Token::from_ticker("WETH").get_addr();
        let weth_buy = buy_mint.to_lowercase() == weth_addr.to_lowercase();

        // If we're deliberately refunding via native ETH, or the buy-side token
        // is native ETH or WETH, we don't need to get a conversion rate
        if refund_native_eth || native_eth_buy || weth_buy {
            return Ok(None);
        }

        // Get ETH price
        let eth_price_f64 = self.price_reporter_client.get_eth_price().await?;
        let eth_price = BigDecimal::from_f64(eth_price_f64)
            .ok_or(AuthServerError::gas_sponsorship("failed to convert ETH price to BigDecimal"))?;

        let buy_token_price = get_nominal_buy_token_price(buy_mint, match_result)?;

        // Compute conversion rate of nominal units of TOKEN per whole ETH
        let conversion_rate = eth_price / buy_token_price;

        // Convert the scaled rate to a U256. We can use the `BigInt` component of the
        // `BigDecimal` directly because we round to 0 digits after the decimal.
        let (conversion_rate_bigint, _) =
            conversion_rate.round(0 /* round_digits */).into_bigint_and_scale();

        let conversion_rate_u256 = AlloyU256::try_from(conversion_rate_bigint)
            .map_err(AuthServerError::gas_sponsorship)?;

        Ok(Some(conversion_rate_u256))
    }

    /// Apply a gas sponsorship refund to a quote.
    /// This method assumes that the refund was in-kind, i.e. that the refund
    /// amount is in terms of the buy-side token.
    pub(crate) fn apply_sponsorship_to_quote(
        &self,
        quote: &mut ApiExternalQuote,
        refund_amount: u128,
    ) -> Result<(), AuthServerError> {
        let (base_amount, quote_amount) = match quote.match_result.direction {
            OrderSide::Buy => {
                (quote.match_result.base_amount + refund_amount, quote.match_result.quote_amount)
            },
            OrderSide::Sell => {
                (quote.match_result.base_amount, quote.match_result.quote_amount + refund_amount)
            },
        };

        let base_amt_f64 = base_amount as f64;
        let quote_amt_f64 = quote_amount as f64;
        let price = quote_amt_f64 / base_amt_f64;

        quote.price.price = price.to_string();
        quote.receive.amount += refund_amount;
        quote.match_result.base_amount = base_amount;
        quote.match_result.quote_amount = quote_amount;

        Ok(())
    }
}
