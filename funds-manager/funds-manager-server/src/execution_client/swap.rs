//! Handlers for executing swaps

use std::{cmp::Ordering, iter};

use alloy_primitives::{Address, TxHash, U256};
use funds_manager_api::{
    quoters::{QuoteParams, SwapIntoTargetTokenRequest},
    u256_try_into_u128,
};
use futures::future::join_all;
use renegade_common::types::token::{get_all_tokens, Token, USDC_TICKER};
use tracing::{info, instrument, warn};

use crate::{
    execution_client::venues::{
        quote::{ExecutableQuote, ExecutionQuote, QuoteExecutionData},
        ExecutionResult, ExecutionVenue,
    },
    metrics::labels::{
        ASSET_TAG, CHAIN_TAG, QUOTE_PRICE_DEVIATION, TRADE_SIDE_FACTOR_TAG, VENUE_TAG,
    },
};

use super::{error::ExecutionClientError, ExecutionClient};

// -------------
// | Constants |
// -------------

/// The factor by which the swap size will be divided when retrying
const SWAP_DECAY_FACTOR: U256 = U256::from_limbs([2, 0, 0, 0]);
/// The minimum amount of USDC that will be attempted to be swapped recursively
const MIN_SWAP_QUOTE_AMOUNT: f64 = 10.0; // 10 USDC
/// The default maximum allowable deviation from the Renegade price in a quote
const DEFAULT_MAX_PRICE_DEVIATION: f64 = 0.0100; // 100bps, or 1%
/// The relative amount by which the price deviation tolerance will be increased
const PRICE_DEVIATION_INCREASE: f64 = 0.2; // 20%
/// The maximum multiple of the default price deviation tolerance that will be
/// allowed
const MAX_PRICE_DEVIATION_INCREASE: f64 = 4.0; // 4x
/// The buffer to scale the target amount by when executing swaps to cover it,
/// to account for price drift
const SWAP_TO_COVER_BUFFER: f64 = 1.1;
/// The default slippage tolerance for a quote
pub const DEFAULT_SLIPPAGE_TOLERANCE: f64 = 0.001; // 10bps

// ---------
// | Types |
// ---------

/// The outcome of a successfully-executed decaying swap
pub struct DecayingSwapOutcome {
    /// The quote that was executed
    pub quote: ExecutionQuote,
    /// The actual amount of the token that was bought
    pub buy_amount_actual: U256,
    /// The transaction hash in which the swap was executed
    pub tx_hash: TxHash,
    /// The cumulative gas cost of the swap, across all attempts.
    pub cumulative_gas_cost: U256,
}

/// A candidate token to swap out of to cover a target amount of another token
struct SwapCandidate {
    /// The candidate token
    pub token: Token,
    /// The balance of the token
    pub balance: f64,
    /// The price of the token
    pub price: f64,
}

impl SwapCandidate {
    /// Compute the notional value of the swap candidate
    pub fn notional_value(&self) -> f64 {
        self.balance * self.price
    }
}

// --------------------
// | Execution Client |
// --------------------

impl ExecutionClient {
    /// Attempt to execute a swap, retrying failed swaps with
    /// decreased quotes down to a minimum trade size.
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
        loop {
            if !self.can_execute_swap(&params).await? {
                return Ok(None);
            }

            let maybe_executable_quote = self.get_best_quote(params.clone()).await?;
            if maybe_executable_quote.is_none() {
                warn!("No quote found for swap");
                return Ok(None);
            }

            let executable_quote = maybe_executable_quote.unwrap();

            if self
                .exceeds_price_deviation(&executable_quote.quote, max_price_deviation_multiplier)
                .await?
            {
                adjust_for_price_deviation(&mut params, &mut max_price_deviation_multiplier);
                if max_price_deviation_multiplier > MAX_PRICE_DEVIATION_INCREASE {
                    warn!("Price deviation tolerance exceeds maximum increase ({MAX_PRICE_DEVIATION_INCREASE}x)");
                    return Ok(None);
                }

                continue;
            }

            // Execute the quote
            let ExecutionResult { buy_amount_actual, gas_cost, tx_hash } =
                self.execute_quote(&executable_quote).await?;

            cumulative_gas_cost += gas_cost;

            // If the swap was successful, return
            if let Some(tx_hash) = tx_hash {
                return Ok(Some(DecayingSwapOutcome {
                    quote: executable_quote.quote,
                    buy_amount_actual,
                    tx_hash,
                    cumulative_gas_cost,
                }));
            }

            // Otherwise, decrease the swap size and try again
            warn!("swap failed, retrying w/ reduced-size quote");
            params.from_amount /= SWAP_DECAY_FACTOR;
        }
    }

    /// Try to execute swaps to cover a target amount of a token,
    /// first checking if any swaps are necessary.
    ///
    /// Returns a vector of outcomes for the executed swaps.
    pub async fn try_swap_into_target_token(
        &self,
        req: SwapIntoTargetTokenRequest,
    ) -> Result<Vec<DecayingSwapOutcome>, ExecutionClientError> {
        let SwapIntoTargetTokenRequest { target_amount, quote_params, exclude_tokens } = req;

        let target_token = Token::from_addr_on_chain(&quote_params.to_token, self.chain);
        let excluded_tokens = exclude_tokens
            .into_iter()
            .map(|t| Token::from_addr_on_chain(&t, self.chain))
            .chain(iter::once(target_token.clone()))
            .collect();

        // Check that the current token balances doesn't already cover the target amount
        let current_balance = self.get_erc20_balance(&target_token.addr).await?;

        if current_balance >= target_amount {
            let ticker = target_token.get_ticker().unwrap_or(target_token.get_addr());
            info!("Current {ticker} balance ({current_balance}) is greater than target amount ({target_amount}), skipping swaps");
            return Ok(vec![]);
        }

        let amount_to_cover = target_amount - current_balance;
        let price = self.price_reporter.get_price(&target_token.addr, self.chain).await?;
        let amount_to_cover_usdc = amount_to_cover * price;

        // Check that the amount to cover is greater than the minimum swap amount
        if amount_to_cover_usdc < MIN_SWAP_QUOTE_AMOUNT {
            info!("Target token value to cover (${amount_to_cover_usdc}) is less than minimum swap amount (${MIN_SWAP_QUOTE_AMOUNT}), skipping swaps");
            return Ok(vec![]);
        }

        self.execute_swaps_into_target_token(
            quote_params,
            target_token,
            amount_to_cover_usdc,
            excluded_tokens,
        )
        .await
    }

    /// Try to execute swaps to cover the target balances of all the input
    /// tokens.
    ///
    /// We do this by first swapping into USDC, excluding the target tokens from
    /// the swap path, and then swapping from USDC into each of the target
    /// tokens.
    ///
    /// We do this so that we can maintain the current semantics around "buying"
    /// & "selling", and what the "base" token in a swap is, when recording
    /// post-swap telemetry.
    ///
    /// The tuples in the `target_tokens` vector should contain the target
    /// token, and the target balance of that token
    pub async fn multi_swap_into_target_tokens(
        &self,
        target_tokens: &[(Token, f64)],
    ) -> Result<Vec<DecayingSwapOutcome>, ExecutionClientError> {
        // Get the final USDC balance needed to cover all the target tokens
        let purchase_values = self.get_multi_swap_purchase_values(target_tokens).await?;

        let mut swap_outcomes = self.buy_usdc_for_multi_swap(&purchase_values).await?;

        // Filter out any zero-value purchases, and any purchases of USDC
        let filtered_purchase_values = purchase_values.into_iter().filter(|(token, value)| {
            let nonzero_purchase = *value > 0.0;
            let non_usdc_purchase =
                token.get_ticker().map(|ticker| ticker != USDC_TICKER).unwrap_or(true);

            nonzero_purchase && non_usdc_purchase
        });

        for (token, purchase_value) in filtered_purchase_values {
            let ticker = token.get_ticker().unwrap_or(token.get_addr());
            match self.buy_token_dollar_amount(token, purchase_value * SWAP_TO_COVER_BUFFER).await {
                Ok(Some(outcome)) => swap_outcomes.push(outcome),
                Ok(None) => warn!("No swap executed for {ticker}"),
                Err(e) => warn!("Error swapping into {ticker}: {e}"),
            }
        }

        Ok(swap_outcomes)
    }

    // ---------------------------------
    // | Target Token Swapping Helpers |
    // ---------------------------------

    /// Execute swaps to cover a target amount of a token.
    ///
    /// Returns a vector of outcomes for the executed swaps.
    async fn execute_swaps_into_target_token(
        &self,
        params: QuoteParams,
        target_token: Token,
        amount_to_cover_usdc: f64,
        excluded_tokens: Vec<Token>,
    ) -> Result<Vec<DecayingSwapOutcome>, ExecutionClientError> {
        let target_ticker = target_token.get_ticker().unwrap_or(target_token.get_addr());

        // Get the balances of the candidate tokens to swap out of,
        // sorted by descending value
        let swap_candidates = self.get_swap_candidates(excluded_tokens).await?;

        // We increase the amount to cover by a fixed buffer to account for drift
        // in the prices sampled when getting swap candidates
        let mut remaining_amount_usdc = amount_to_cover_usdc * SWAP_TO_COVER_BUFFER;
        info!("Need to cover ${amount_to_cover_usdc} {target_ticker}, purchasing ${remaining_amount_usdc}");

        let mut outcomes = vec![];
        for candidate in swap_candidates {
            if remaining_amount_usdc < MIN_SWAP_QUOTE_AMOUNT {
                info!("Remaining amount to cover (${remaining_amount_usdc}) is less than minimum swap amount (${MIN_SWAP_QUOTE_AMOUNT}), stopping swaps");
                break;
            }

            let token = &candidate.token;
            let ticker = token.get_ticker().unwrap_or(token.get_addr());

            let maybe_outcome = self
                .try_swap_candidate(
                    params.clone(),
                    target_token.get_addr(),
                    candidate,
                    remaining_amount_usdc,
                )
                .await?;

            if maybe_outcome.is_none() {
                continue;
            }

            let swap_outcome = maybe_outcome.unwrap();
            let sell_amount = swap_outcome.quote.sell_amount_decimal();
            let buy_amount = u256_try_into_u128(swap_outcome.buy_amount_actual)
                .map_err(ExecutionClientError::parse)?;

            let buy_amount_decimal = swap_outcome.quote.buy_token.convert_to_decimal(buy_amount);

            outcomes.push(swap_outcome);

            info!("Swapped {sell_amount} {ticker} for {buy_amount_decimal} {target_ticker}");

            remaining_amount_usdc -= buy_amount_decimal;
        }

        Ok(outcomes)
    }

    /// Get the candidate token balances to swap out of to cover some amount of
    /// the target token. Returns a vector of (token, balance, price)
    /// tuples, sorted by descending value.
    async fn get_swap_candidates(
        &self,
        excluded_tokens: Vec<Token>,
    ) -> Result<Vec<SwapCandidate>, ExecutionClientError> {
        let candidate_tokens: Vec<Token> = get_all_tokens()
            .into_iter()
            .filter(|token| self.swap_candidate_predicate(token, &excluded_tokens))
            .collect();

        let mut swap_candidates = vec![];
        for token in candidate_tokens {
            let balance = self.get_erc20_balance(&token.addr).await?;
            let price = self.price_reporter.get_price(&token.addr, self.chain).await?;
            swap_candidates.push(SwapCandidate { token, balance, price });
        }

        // Sort the tokens by their value, descending
        swap_candidates.sort_by(|a, b| {
            let value_a = a.notional_value();
            let value_b = b.notional_value();

            value_b.partial_cmp(&value_a).unwrap_or(Ordering::Equal)
        });

        Ok(swap_candidates)
    }

    /// A predicate for filtering candidate tokens to swap into the target token
    fn swap_candidate_predicate(&self, token: &Token, excluded_tokens: &[Token]) -> bool {
        let token_on_chain = token.get_chain() == self.chain;
        let token_not_excluded = !excluded_tokens.contains(token);
        let token_not_stablecoin = !token.is_stablecoin();
        let token_not_usd_mock = token.get_addr() != Address::ZERO.to_string();

        token_on_chain && token_not_excluded && token_not_stablecoin && token_not_usd_mock
    }

    /// Try to swap out of a candidate token to cover a target amount of a
    /// token.
    ///
    /// Returns an outcome for the swap if it was successful,
    /// or `None` if no swap occurred.
    async fn try_swap_candidate(
        &self,
        params: QuoteParams,
        target_token_addr: String,
        candidate: SwapCandidate,
        amount_to_cover_usdc: f64,
    ) -> Result<Option<DecayingSwapOutcome>, ExecutionClientError> {
        let balance_value = candidate.notional_value();
        let SwapCandidate { token, balance, price } = candidate;

        // If the token balance is less than the remaining amount, we swap out of the
        // entire balance. Otherwise, we calculate the necessary amount to
        // swap out of.
        let swap_amount_decimal = if balance_value <= amount_to_cover_usdc {
            balance
        } else {
            amount_to_cover_usdc / price
        };

        let swap_value = swap_amount_decimal * price;
        if swap_value < MIN_SWAP_QUOTE_AMOUNT {
            return Ok(None);
        }

        let swap_amount = token.convert_from_decimal(swap_amount_decimal);
        let swap_params = QuoteParams {
            to_token: target_token_addr,
            from_token: token.get_addr(),
            from_amount: U256::from(swap_amount),
            ..params
        };

        // If there was an error in executing the candidate swap, we return `None` so
        // that we can continue attempting swaps across the other candidates.
        let swap_outcome = self.swap_immediate_decaying(swap_params).await.ok().flatten();
        Ok(swap_outcome)
    }

    /// Get the USDC value of the amount to purchase to cover the target balance
    /// for each input token
    async fn get_multi_swap_purchase_values(
        &self,
        target_tokens: &[(Token, f64)],
    ) -> Result<Vec<(Token, f64)>, ExecutionClientError> {
        let mut purchase_values = Vec::new();

        for (token, target_balance) in target_tokens {
            let price = self.price_reporter.get_price(&token.addr, self.chain).await?;
            let current_balance = self.get_erc20_balance(&token.addr).await?;
            let purchase_amount = (target_balance - current_balance).max(0.0);
            let purchase_value = purchase_amount * price;
            purchase_values.push((token.clone(), purchase_value));
        }

        Ok(purchase_values)
    }

    /// Buy USDC such that we can cover the purchases of all input tokens
    async fn buy_usdc_for_multi_swap(
        &self,
        purchase_values: &[(Token, f64)],
    ) -> Result<Vec<DecayingSwapOutcome>, ExecutionClientError> {
        let usdc_token = Token::from_ticker_on_chain(USDC_TICKER, self.chain);

        let usdc_target_balance = purchase_values.iter().map(|(_, value)| value).sum();
        let exclude_tokens = purchase_values.iter().map(|(token, _)| token.get_addr()).collect();

        let swap_request = SwapIntoTargetTokenRequest {
            target_amount: usdc_target_balance,
            quote_params: QuoteParams {
                to_token: usdc_token.get_addr(),
                from_token: Address::ZERO.to_string(),
                ..Default::default()
            },
            exclude_tokens,
        };

        info!("Buying into {usdc_target_balance} USDC to cover multi-swap purchases");

        self.try_swap_into_target_token(swap_request).await
    }

    /// Buy the given dollar value of a token, using default quote params
    async fn buy_token_dollar_amount(
        &self,
        token: Token,
        purchase_value: f64,
    ) -> Result<Option<DecayingSwapOutcome>, ExecutionClientError> {
        let ticker = token.get_ticker().unwrap_or(token.get_addr());
        info!("Buying ${purchase_value} of {ticker}");

        let usdc_token = Token::from_ticker_on_chain(USDC_TICKER, self.chain);
        let from_amount_u128 = usdc_token.convert_from_decimal(purchase_value);
        let from_amount = from_amount_u128.try_into().map_err(ExecutionClientError::parse)?;

        let swap_params = QuoteParams {
            from_token: usdc_token.get_addr(),
            to_token: token.get_addr(),
            from_amount,
            ..Default::default()
        };

        self.swap_immediate_decaying(swap_params).await
    }

    // ----------------------------
    // | General Swapping Helpers |
    // ----------------------------

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

    /// Get the best quote for a swap, across all execution venues
    #[instrument(
        skip_all,
        fields(
            from_token = %params.from_token,
            to_token = %params.to_token,
            from_amount = %params.from_amount
        )
    )]
    async fn get_best_quote(
        &self,
        params: QuoteParams,
    ) -> Result<Option<ExecutableQuote>, ExecutionClientError> {
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
                let quote_res = venue.get_quote(params).await;
                (venue, quote_res)
            }
        });
        let quote_results = join_all(quote_futures).await;

        let mut maybe_best_quote = None;
        for (venue, quote_res) in quote_results {
            let venue_specifier = venue.venue_specifier();
            if let Err(e) = quote_res {
                warn!("Error getting quote from {venue_specifier}: {e}");
                continue;
            }

            let quote = quote_res.unwrap();
            let quote_price = quote.quote.get_price(None /* buy_amount */);
            let is_sell = quote.quote.is_sell();

            info!("{venue_specifier} quote price: {quote_price} (is_sell: {is_sell})");

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
        executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError> {
        match executable_quote.execution_data {
            QuoteExecutionData::Lifi(_) => self.venues.lifi.execute_quote(executable_quote).await,
            QuoteExecutionData::Cowswap(_) => {
                self.venues.cowswap.execute_quote(executable_quote).await
            },
            QuoteExecutionData::Bebop(_) => self.venues.bebop.execute_quote(executable_quote).await,
        }
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

/// Adjust the given quote params in the case that the resulting quote exceeds
/// the price deviation tolerance.
///
/// Concretely, this means increasing the max price deviation multiplier if the
/// params allow for it, or reducing the swap size otherwise.
fn adjust_for_price_deviation(params: &mut QuoteParams, max_price_deviation_multiplier: &mut f64) {
    if params.increase_price_deviation {
        *max_price_deviation_multiplier += PRICE_DEVIATION_INCREASE;
        info!("Price deviation exceeded, increasing price deviation tolerance by {max_price_deviation_multiplier}x");
    } else {
        info!("Price deviation exceeded, reducing swap size by {SWAP_DECAY_FACTOR}x");
        params.from_amount /= SWAP_DECAY_FACTOR;
    }
}
