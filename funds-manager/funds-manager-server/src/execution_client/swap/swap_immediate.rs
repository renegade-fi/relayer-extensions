//! Logic & helpers for immediate swap functionality

use alloy_primitives::U256;
use funds_manager_api::{quoters::QuoteParams, u256_try_into_u128};
use futures::future::join_all;
use renegade_common::types::token::Token;
use tracing::instrument;

use crate::log_task;
use crate::logger::{Outcome, Task};

use crate::{
    execution_client::{
        error::ExecutionClientError,
        swap::{DecayingSwapOutcome, MIN_SWAP_QUOTE_AMOUNT},
        venues::{
            quote::{CrossVenueQuoteSource, ExecutableQuote, ExecutionQuote, QuoteExecutionData},
            ExecutionResult, ExecutionVenue,
        },
        ExecutionClient,
    },
    metrics::labels::{
        ASSET_TAG, CHAIN_TAG, QUOTE_PRICE_DEVIATION, TRADE_SIDE_FACTOR_TAG, VENUE_TAG,
    },
};

// -------------
// | Constants |
// -------------

/// The factor by which the swap size will be divided when retrying
const SWAP_DECAY_FACTOR: U256 = U256::from_limbs([2, 0, 0, 0]);
/// The default maximum allowable deviation from the Renegade price in a quote
const DEFAULT_MAX_PRICE_DEVIATION: f64 = 0.0100; // 100bps, or 1%
/// The relative amount by which the price deviation tolerance will be increased
const PRICE_DEVIATION_INCREASE: f64 = 0.2; // 20%
/// The maximum multiple of the default price deviation tolerance that will be
/// allowed
const MAX_PRICE_DEVIATION_INCREASE: f64 = 4.0; // 4x
/// The maximum number of times we'll retry a swap by excluding the failing
/// quote source before falling back to decaying the swap size
const MAX_RETRIES_WITH_EXCLUSION: usize = 5;

// ---------
// | Types |
// ---------

/// The unsuccessful control flow branches that can occur during the execution
/// of a decaying swap
#[derive(Debug, Clone)]
enum SwapControlFlow {
    /// Break out of the loop with no swap outcome
    NoSwap,
    /// Continue the loop with a higher price deviation tolerance
    IncreasePriceDeviation,
    /// Continue the loop with a smaller swap size.
    DecreaseSwapSize {
        /// The gas cost incurred in the swap attempt.
        gas_cost: U256,
    },
    /// Continue the loop while excluding quotes from the given source
    ExcludeQuoteSource {
        /// The source of the quote to exclude
        source: CrossVenueQuoteSource,
        /// The gas cost incurred in the swap attempt.
        gas_cost: U256,
    },
    /// Break out of the loop with an error
    Error(ExecutionClientError),
}

impl From<ExecutionClientError> for SwapControlFlow {
    fn from(e: ExecutionClientError) -> Self {
        SwapControlFlow::Error(e)
    }
}

impl ExecutionClient {
    /// Attempt to execute a swap, with the following control flow:
    /// 1. Fetch quotes from all sources (see `CrossVenueQuoteSource`) across
    ///    all venues, unless an individual venue is specified in the params.
    /// 2. Select the best quote from those fetched.
    /// 3. If the quote exceeds the max deviation from reference price, retry
    ///    from step 1 with a decreased swap size, unless the
    ///    `increase_price_deviation` flag is set, in which case we retry with a
    ///    higher deviation tolerance.
    /// 4. Execute the quote, and if the swap fails, retry from step 1, but
    ///    exclude the failed quote's source from subsequent quote fetches.
    /// 5. After the maximum number of retries w/ source exclusion is reached,
    ///    subsequent retries will be attempted with a decreased swap size.
    ///
    /// Returns the quote, transaction receipt, and cumulative gas cost of all
    /// attempted swaps
    #[instrument(
        skip_all,
        fields(
            from_token = %params.from_token,
            to_token = %params.to_token,
            from_amount = %params.from_amount
        )
    )]
    pub async fn swap_immediate_decaying(
        &self,
        mut params: QuoteParams,
    ) -> Result<Option<DecayingSwapOutcome>, ExecutionClientError> {
        let mut cumulative_gas_cost = U256::ZERO;
        let mut max_price_deviation_multiplier = 1.0;
        let mut excluded_quote_sources = Vec::new();
        let mut num_swaps_with_exclusion = 0;
        loop {
            match self
                .execute_swap_step(
                    &params,
                    max_price_deviation_multiplier,
                    cumulative_gas_cost,
                    &excluded_quote_sources,
                    &mut num_swaps_with_exclusion,
                )
                .await
            {
                Ok(outcome) => return Ok(Some(outcome)),
                Err(SwapControlFlow::NoSwap) => return Ok(None),
                Err(SwapControlFlow::Error(e)) => return Err(e),
                Err(SwapControlFlow::IncreasePriceDeviation) => {
                    max_price_deviation_multiplier += PRICE_DEVIATION_INCREASE;
                    log_task!(
                        Task::Swap,
                        Outcome::Retrying,
                        multiplier = max_price_deviation_multiplier,
                        "increasing price deviation tolerance to {max_price_deviation_multiplier}x"
                    );

                    if max_price_deviation_multiplier > MAX_PRICE_DEVIATION_INCREASE {
                        log_task!(
                            Task::Swap,
                            Outcome::Failed,
                            multiplier = max_price_deviation_multiplier,
                            max_multiplier = MAX_PRICE_DEVIATION_INCREASE,
                            "price deviation tolerance exceeds maximum increase ({MAX_PRICE_DEVIATION_INCREASE}x)"
                        );
                        return Ok(None);
                    }
                },
                Err(SwapControlFlow::DecreaseSwapSize { gas_cost }) => {
                    log_task!(
                        Task::Swap,
                        Outcome::Retrying,
                        decay_factor = %SWAP_DECAY_FACTOR,
                        "decreasing swap size by {SWAP_DECAY_FACTOR}x"
                    );
                    params.from_amount /= SWAP_DECAY_FACTOR;
                    cumulative_gas_cost += gas_cost;

                    // Decreasing the swap size constitutes a meaningful change in the quote
                    // parameters. As such, we reset the excluded quote sources
                    // and the number of swaps with exclusion.
                    excluded_quote_sources.clear();
                    num_swaps_with_exclusion = 0;
                },
                Err(SwapControlFlow::ExcludeQuoteSource { source, gas_cost }) => {
                    cumulative_gas_cost += gas_cost;
                    excluded_quote_sources.push(source);
                },
            }
        }
    }

    // -----------
    // | Helpers |
    // -----------

    /// Executes a single step of a decaying swap, returning the outcome if the
    /// swap was successful, and otherwise the control flow branch to take.
    async fn execute_swap_step(
        &self,
        params: &QuoteParams,
        max_price_deviation_multiplier: f64,
        cumulative_gas_cost: U256,
        excluded_quote_sources: &[CrossVenueQuoteSource],
        num_swaps_with_exclusion: &mut usize,
    ) -> Result<DecayingSwapOutcome, SwapControlFlow> {
        let executable_quote = self
            .get_executable_quote(params, max_price_deviation_multiplier, excluded_quote_sources)
            .await?;

        self.execute_quote(executable_quote, cumulative_gas_cost, num_swaps_with_exclusion).await
    }

    /// Gets an executable quote for a swap, validating the preconditions for
    /// fetching the quote and the quote itself.
    async fn get_executable_quote(
        &self,
        params: &QuoteParams,
        max_price_deviation_multiplier: f64,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<ExecutableQuote, SwapControlFlow> {
        if !self.can_execute_swap(params).await? {
            return Err(SwapControlFlow::NoSwap);
        }

        let maybe_executable_quote = self.fetch_best_quote(params, excluded_quote_sources).await?;
        if maybe_executable_quote.is_none() {
            log_task!(Task::FetchQuote, Outcome::Failed, "no quote found for swap");
            return Err(SwapControlFlow::NoSwap);
        }

        let executable_quote = maybe_executable_quote.unwrap();

        if self
            .exceeds_price_deviation(&executable_quote.quote, max_price_deviation_multiplier)
            .await?
        {
            if params.increase_price_deviation {
                return Err(SwapControlFlow::IncreasePriceDeviation);
            }

            return Err(SwapControlFlow::DecreaseSwapSize { gas_cost: U256::ZERO });
        }

        Ok(executable_quote)
    }

    /// Check whether a swap represented by the quote params meets the criteria
    /// for execution
    async fn can_execute_swap(&self, params: &QuoteParams) -> Result<bool, ExecutionClientError> {
        if !self.has_sufficient_balance(params).await? {
            log_task!(
                Task::Swap,
                Outcome::Skipped,
                "hot wallet does not have sufficient balance to cover swap"
            );
            return Ok(false);
        }

        let expected_quote_amount = self.get_expected_quote_amount(params).await?;
        if expected_quote_amount < MIN_SWAP_QUOTE_AMOUNT {
            log_task!(
                Task::Swap,
                Outcome::Skipped,
                expected_amount = expected_quote_amount,
                min_amount = MIN_SWAP_QUOTE_AMOUNT,
                "expected swap amount of {expected_quote_amount} USDC is less than minimum swap amount ({MIN_SWAP_QUOTE_AMOUNT})"
            );
            return Ok(false);
        }

        Ok(true)
    }

    /// Compute the expected quote amount for a swap, using the Renegade price
    /// for the sell token
    async fn get_expected_quote_amount(
        &self,
        params: &QuoteParams,
    ) -> Result<f64, ExecutionClientError> {
        let from_token = Token::from_addr_on_chain(&params.from_token, self.chain);
        let from_amount_u128 =
            u256_try_into_u128(params.from_amount).map_err(ExecutionClientError::parse)?;

        let from_amount_f64 = from_token.convert_to_decimal(from_amount_u128);
        if from_token.is_stablecoin() {
            return Ok(from_amount_f64);
        }

        let price = self.price_reporter.get_price(&from_token.addr, self.chain).await?;
        let approx_quote_amount = from_amount_f64 * price;

        Ok(approx_quote_amount)
    }

    /// Fetch the best quote for a swap, across all execution venues
    #[instrument(
        skip_all,
        fields(
            from_token = %params.from_token,
            to_token = %params.to_token,
            from_amount = %params.from_amount
        )
    )]
    async fn fetch_best_quote(
        &self,
        params: &QuoteParams,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<Option<ExecutableQuote>, ExecutionClientError> {
        let all_quotes = self.fetch_all_quotes(params, excluded_quote_sources).await?;

        let valid_quotes = all_quotes.into_iter().filter(|quote| quote.is_valid()).collect();

        self.select_best_quote(valid_quotes)
    }

    /// Fetch quotes across all venues
    async fn fetch_all_quotes(
        &self,
        params: &QuoteParams,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<Vec<ExecutableQuote>, ExecutionClientError> {
        // If a venue is specified in the params, we only consider that venue
        let venues = if let Some(venue) = params.venue {
            match self.venues.get_venue(venue) {
                Some(v) => vec![v],
                None => {
                    return Err(ExecutionClientError::custom(format!(
                        "venue {venue:?} requested but not configured"
                    )));
                },
            }
        } else {
            self.venues.get_all_venues()
        };

        // Fetch all quotes in parallel
        let quote_futures = venues.into_iter().map(|venue| {
            let params = params.clone();
            async move {
                let quote_res = venue.get_quotes(params, excluded_quote_sources).await;
                (venue, quote_res)
            }
        });
        let quote_results = join_all(quote_futures).await;

        let mut all_quotes = Vec::new();
        for (venue, quotes_res) in quote_results {
            if let Err(e) = quotes_res {
                let venue_specifier = venue.venue_specifier();
                log_task!(
                    Task::FetchQuote,
                    Outcome::Partial,
                    venue = %venue_specifier,
                    error = %e,
                    "error getting quote from {venue_specifier}: {e}"
                );
                continue;
            }

            let quotes = quotes_res.unwrap();
            all_quotes.extend(quotes);
        }

        Ok(all_quotes)
    }

    /// Select the best quote from a list of quotes
    fn select_best_quote(
        &self,
        all_quotes: Vec<ExecutableQuote>,
    ) -> Result<Option<ExecutableQuote>, ExecutionClientError> {
        let mut maybe_best_quote = None;
        for quote in all_quotes {
            let quote_price = quote.quote.get_price(None /* buy_amount */);
            let is_sell = quote.quote.is_sell();
            let quote_source = &quote.quote.source;

            log_task!(
                Task::FetchQuote,
                Outcome::Ok,
                source = %quote_source,
                quote_price = quote_price,
                is_sell = is_sell,
                "{quote_source} quote price: {quote_price} (is_sell: {is_sell})"
            );

            if maybe_best_quote.is_none() {
                maybe_best_quote = Some(quote);
                continue;
            }

            let best_quote = maybe_best_quote.as_ref().unwrap();
            let best_quote_price = best_quote.quote.get_price(None /* buy_amount */);

            let is_better_sell = is_sell && quote_price > best_quote_price;
            let is_better_buy = !is_sell && quote_price < best_quote_price;

            if is_better_sell || is_better_buy {
                maybe_best_quote = Some(quote);
            }
        }

        Ok(maybe_best_quote)
    }

    /// Check whether the hot wallet has a sufficient balance to cover a swap
    /// represened by the quote params
    async fn has_sufficient_balance(
        &self,
        params: &QuoteParams,
    ) -> Result<bool, ExecutionClientError> {
        let balance = self.get_erc20_balance_raw(&params.from_token).await?;
        Ok(balance >= params.from_amount)
    }

    /// Check if a quote deviates too far from the Renegade price.
    ///
    /// The check is two-sided: a venue quote that is much *better* than the
    /// reference price is also rejected, on the assumption that the reference
    /// price itself may be wrong. See incident 2026-05-08 (cbBTC).
    async fn exceeds_price_deviation(
        &self,
        quote: &ExecutionQuote,
        max_deviation_multiplier: f64,
    ) -> Result<bool, ExecutionClientError> {
        // Get the renegade price for the pair
        let base_addr = &quote.base_token().addr;
        let renegade_price = self.price_reporter.get_price(base_addr, quote.chain).await?;

        let quote_price = quote.get_price(None /* buy_amount */);

        let max_deviation = quote
            .base_token()
            .get_ticker()
            .and_then(|ticker| self.max_price_deviations.get(&ticker).copied())
            .unwrap_or(DEFAULT_MAX_PRICE_DEVIATION);

        let deviation_threshold = max_deviation * max_deviation_multiplier;

        let (deviation, exceeds_max_deviation) = compute_price_deviation(
            quote.is_sell(),
            quote_price,
            renegade_price,
            deviation_threshold,
        );

        // Record the price deviation regardless of whether it exceeds the threshold.
        // This metric is useful for tuning the deviation maximums.
        record_price_deviation(quote, deviation);
        if exceeds_max_deviation {
            log_task!(
                Task::FetchQuote,
                Outcome::Partial,
                quote_price = quote_price,
                renegade_price = renegade_price,
                deviation = deviation,
                deviation_threshold = deviation_threshold,
                "quote deviates too far from the Renegade price"
            );
        }

        Ok(exceeds_max_deviation)
    }

    /// Execute a quote on the associated venue
    async fn execute_quote(
        &self,
        executable_quote: ExecutableQuote,
        mut cumulative_gas_cost: U256,
        num_swaps_with_exclusion: &mut usize,
    ) -> Result<DecayingSwapOutcome, SwapControlFlow> {
        let ExecutionResult { buy_amount_actual, gas_cost, tx_hash } =
            match executable_quote.execution_data {
                QuoteExecutionData::Lifi(_) => {
                    self.venues.lifi.execute_quote(&executable_quote).await?
                },
                QuoteExecutionData::Cowswap(_) => {
                    self.venues.cowswap.execute_quote(&executable_quote).await?
                },
                QuoteExecutionData::Bebop(_) => {
                    self.venues.bebop.execute_quote(&executable_quote).await?
                },
                QuoteExecutionData::Okx(_) => {
                    let okx = self.venues.okx.as_ref().ok_or_else(|| {
                        ExecutionClientError::custom(
                            "OKX quote received but OKX venue is not configured",
                        )
                    })?;
                    okx.execute_quote(&executable_quote).await?
                },
            };

        cumulative_gas_cost += gas_cost;

        // If the swap was successful, return
        if let Some(tx_hash) = tx_hash {
            return Ok(DecayingSwapOutcome {
                quote: executable_quote.quote,
                buy_amount_actual,
                tx_hash,
                cumulative_gas_cost,
            });
        }

        *num_swaps_with_exclusion += 1;

        if *num_swaps_with_exclusion < MAX_RETRIES_WITH_EXCLUSION {
            // We first retry the swap by excluding the failing quote source
            return Err(SwapControlFlow::ExcludeQuoteSource {
                source: executable_quote.quote.source,
                gas_cost,
            });
        }

        // Once the maximum number of retries via source exclusion is reached,
        // we fall back to decaying the swap size
        Err(SwapControlFlow::DecreaseSwapSize { gas_cost })
    }
}

/// Record a quote's price deviation from the Renegade price
fn record_price_deviation(quote: &ExecutionQuote, deviation: f64) {
    let base_token = quote.base_token();
    let asset = base_token.get_ticker().unwrap_or(base_token.get_addr());

    metrics::gauge!(
        QUOTE_PRICE_DEVIATION,
        CHAIN_TAG => quote.chain.to_string(),
        ASSET_TAG => asset,
        TRADE_SIDE_FACTOR_TAG => if quote.is_sell() { "sell" } else { "buy" },
        VENUE_TAG => quote.venue.to_string(),
    )
    .set(deviation);
}

/// Compute the signed price deviation of a venue quote against a reference
/// price, and whether it exceeds the threshold in either direction.
///
/// The signed convention is "positive = worse for the protocol": for a sell,
/// `(reference - quote) / reference` is positive when the venue is selling our
/// asset cheaply; for a buy, `(quote - reference) / reference` is positive
/// when the venue is charging us a premium. The exceedance check is two-sided
/// (`abs`) so that a venue quote that is suspiciously *favorable* against the
/// reference is also rejected — the reference price itself may be wrong (see
/// incident 2026-05-08, cbBTC).
///
/// The check fails closed for non-finite deviations: a NaN reference or
/// quote price, or a zero reference, produces a NaN/inf deviation that is
/// treated as exceeding the threshold rather than passing it.
fn compute_price_deviation(
    is_sell: bool,
    quote_price: f64,
    reference_price: f64,
    deviation_threshold: f64,
) -> (f64, bool) {
    let deviation = if is_sell {
        (reference_price - quote_price) / reference_price
    } else {
        (quote_price - reference_price) / reference_price
    };
    let abs_dev = deviation.abs();
    let exceeds = !abs_dev.is_finite() || abs_dev > deviation_threshold;
    (deviation, exceeds)
}

#[cfg(test)]
mod tests {
    use super::compute_price_deviation;

    /// 1% — matches `DEFAULT_MAX_PRICE_DEVIATION`.
    const THRESHOLD: f64 = 0.01;

    #[test]
    fn sell_at_venue_below_reference_is_rejected() {
        // Venue offers $98 to sell when reference says $100 — 2% unfavorable.
        let (deviation, exceeds) = compute_price_deviation(true, 98.0, 100.0, THRESHOLD);
        assert!(exceeds);
        assert!(deviation > 0.0, "unfavorable deviation should be positive");
    }

    #[test]
    fn buy_at_venue_above_reference_is_rejected() {
        // Venue charges $102 to buy when reference says $100 — 2% unfavorable.
        let (deviation, exceeds) = compute_price_deviation(false, 102.0, 100.0, THRESHOLD);
        assert!(exceeds);
        assert!(deviation > 0.0, "unfavorable deviation should be positive");
    }

    #[test]
    fn sell_at_venue_above_reference_is_rejected_by_abs() {
        // Venue offers $200 to sell when reference says $100 — favorable by 100%,
        // but the reference is suspect. Pre-fix this passed; the two-sided check
        // catches it.
        let (deviation, exceeds) = compute_price_deviation(true, 200.0, 100.0, THRESHOLD);
        assert!(exceeds, "two-sided gate must reject suspect-favorable sell quote");
        assert!(deviation < 0.0, "favorable deviation is negative for sells");
    }

    #[test]
    fn buy_at_venue_below_reference_is_rejected_by_abs() {
        // Venue charges $50 to buy when reference says $100 — favorable by 50%.
        let (deviation, exceeds) = compute_price_deviation(false, 50.0, 100.0, THRESHOLD);
        assert!(exceeds, "two-sided gate must reject suspect-favorable buy quote");
        assert!(deviation < 0.0, "favorable deviation is negative for buys");
    }

    #[test]
    fn within_threshold_passes_either_direction() {
        let small = THRESHOLD / 2.0;
        for is_sell in [true, false] {
            for sign in [1.0, -1.0] {
                let quote_price = 100.0 * (1.0 + sign * small);
                let (_, exceeds) = compute_price_deviation(is_sell, quote_price, 100.0, THRESHOLD);
                assert!(!exceeds, "is_sell={is_sell} sign={sign} should not exceed threshold");
            }
        }
    }

    #[test]
    fn cbbtc_2026_05_08_incident_is_caught() {
        // Reproduces the 2026-05-08 cbBTC scenario: the price reporter returned
        // ~$39,837 (half of real BTC) while Bebop quoted ~$79,256 on a sell.
        // Pre-fix the one-sided check passed (deviation ≈ -0.989 < threshold).
        // Post-fix the absolute value (≈ 0.989) exceeds any reasonable threshold.
        let (deviation, exceeds) = compute_price_deviation(true, 79_256.0, 39_837.0, THRESHOLD);
        assert!(exceeds, "incident scenario must be caught by the .abs() check");
        assert!(deviation < -0.9, "expected ≈ -0.989, got {deviation}");
    }

    #[test]
    fn nan_reference_fails_closed() {
        // A NaN reference price (e.g. from a malformed price-reporter response)
        // must trip the gate. The `!(abs <= threshold)` form is NaN-safe; the
        // older `abs > threshold` form would return false and let the trade
        // proceed against an undefined reference.
        let (_, exceeds_sell) = compute_price_deviation(true, 100.0, f64::NAN, THRESHOLD);
        let (_, exceeds_buy) = compute_price_deviation(false, 100.0, f64::NAN, THRESHOLD);
        assert!(exceeds_sell, "NaN reference must trip the gate (sell)");
        assert!(exceeds_buy, "NaN reference must trip the gate (buy)");
    }

    #[test]
    fn nan_quote_fails_closed() {
        // Symmetric to the above — a NaN venue quote must also trip the gate.
        let (_, exceeds_sell) = compute_price_deviation(true, f64::NAN, 100.0, THRESHOLD);
        let (_, exceeds_buy) = compute_price_deviation(false, f64::NAN, 100.0, THRESHOLD);
        assert!(exceeds_sell);
        assert!(exceeds_buy);
    }

    #[test]
    fn zero_reference_fails_closed() {
        // A zero reference price (e.g. from a corrupted Coinbase order book
        // before the 0-price filter fix) produces an infinite deviation,
        // which the gate must trip.
        let (deviation, exceeds) = compute_price_deviation(true, 100.0, 0.0, THRESHOLD);
        assert!(exceeds);
        assert!(deviation.is_infinite());
    }
}
