//! Logic & helpers for immediate swap functionality

use alloy_primitives::U256;
use funds_manager_api::{quoters::QuoteParams, u256_try_into_u128};
use futures::future::join_all;
use renegade_common::types::token::Token;
use tracing::{info, instrument, warn};

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
                    info!(
                        "Increasing price deviation tolerance to {max_price_deviation_multiplier}x"
                    );

                    if max_price_deviation_multiplier > MAX_PRICE_DEVIATION_INCREASE {
                        warn!("Price deviation tolerance exceeds maximum increase ({MAX_PRICE_DEVIATION_INCREASE}x)");
                        return Ok(None);
                    }
                },
                Err(SwapControlFlow::DecreaseSwapSize { gas_cost }) => {
                    info!("Decreasing swap size by {SWAP_DECAY_FACTOR}x");
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
            warn!("No quote found for swap");
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
            warn!("Hot wallet does not have sufficient balance to cover swap");
            return Ok(false);
        }

        let expected_quote_amount = self.get_expected_quote_amount(params).await?;
        if expected_quote_amount < MIN_SWAP_QUOTE_AMOUNT {
            warn!("Expected swap amount of {expected_quote_amount} USDC is less than minimum swap amount ({MIN_SWAP_QUOTE_AMOUNT})");
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

        self.select_best_quote(all_quotes)
    }

    /// Fetch quotes across all venues
    async fn fetch_all_quotes(
        &self,
        params: &QuoteParams,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<Vec<ExecutableQuote>, ExecutionClientError> {
        // If a venue is specified in the params, we only consider that venue
        let venues = if let Some(venue) = params.venue {
            vec![self.venues.get_venue(venue)]
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
                warn!("Error getting quote from {venue_specifier}: {e}");
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

            info!("{quote_source} quote price: {quote_price} (is_sell: {is_sell})");

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

    /// Check if a quote deviates too far from the Renegade price
    async fn exceeds_price_deviation(
        &self,
        quote: &ExecutionQuote,
        max_deviation_multiplier: f64,
    ) -> Result<bool, ExecutionClientError> {
        // Get the renegade price for the pair
        let base_addr = &quote.base_token().addr;
        let renegade_price = self.price_reporter.get_price(base_addr, quote.chain).await?;

        let quote_price = quote.get_price(None /* buy_amount */);

        // Check that the price is within the max price impact
        let deviation = if quote.is_sell() {
            (renegade_price - quote_price) / renegade_price
        } else {
            (quote_price - renegade_price) / renegade_price
        };

        // Record the price deviation regardless of whether it exceeds the threshold.
        // This metric is useful for tuning the deviation maximums.
        record_price_deviation(quote, deviation);

        let max_deviation = quote
            .base_token()
            .get_ticker()
            .and_then(|ticker| self.max_price_deviations.get(&ticker).copied())
            .unwrap_or(DEFAULT_MAX_PRICE_DEVIATION);

        let deviation_threshold = max_deviation * max_deviation_multiplier;

        let exceeds_max_deviation = deviation > deviation_threshold;
        if exceeds_max_deviation {
            warn!(
                quote_price,
                renegade_price,
                deviation,
                deviation_threshold,
                "Quote deviates too far from the Renegade price"
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
        let ExecutionResult { buy_amount_actual, gas_cost, tx_hash } = match executable_quote
            .execution_data
        {
            QuoteExecutionData::Lifi(_) => {
                self.venues.lifi.execute_quote(&executable_quote).await?
            },
            QuoteExecutionData::Cowswap(_) => {
                self.venues.cowswap.execute_quote(&executable_quote).await?
            },
            QuoteExecutionData::Bebop(_) => {
                self.venues.bebop.execute_quote(&executable_quote).await?
            },
            QuoteExecutionData::Okx(_) => self.venues.okx.execute_quote(&executable_quote).await?,
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
