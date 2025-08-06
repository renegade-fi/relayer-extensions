//! Handlers for executing swaps

use std::cmp::Ordering;

use alloy_primitives::{Address, TxHash, U256};
use funds_manager_api::{
    quoters::{QuoteParams, SwapIntoTargetTokenRequest},
    u256_try_into_u128,
};
use futures::future::join_all;
use renegade_common::types::token::{get_all_tokens, Token};
use tracing::{info, instrument, warn};

use crate::execution_client::venues::{
    quote::{ExecutableQuote, ExecutionQuote, QuoteExecutionData},
    ExecutionResult, ExecutionVenue,
};

use super::{error::ExecutionClientError, ExecutionClient};

// -------------
// | Constants |
// -------------

/// The factor by which the swap size will be divided when retrying
const SWAP_DECAY_FACTOR: U256 = U256::from_limbs([2, 0, 0, 0]);
/// The minimum amount of USDC that will be attempted to be swapped recursively
const MIN_SWAP_QUOTE_AMOUNT: f64 = 10.0; // 10 USDC
/// The maximum price deviation from the Renegade price that is allowed
const MAX_PRICE_DEVIATION: f64 = 0.02; // 2%
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
    pub async fn swap_immediate_decaying(
        &self,
        mut params: QuoteParams,
    ) -> Result<Option<DecayingSwapOutcome>, ExecutionClientError> {
        let mut cumulative_gas_cost = U256::ZERO;
        loop {
            // The from amount may have decayed to zero,
            // in which case fetching a quote is impossible
            if params.from_amount == U256::ZERO {
                return Ok(None);
            }

            let maybe_executable_quote = self.get_best_quote(params.clone()).await?;
            if maybe_executable_quote.is_none() {
                warn!("No quote found for swap across all venues");
                return Ok(None);
            }

            let executable_quote = maybe_executable_quote.unwrap();
            let quote_amount = executable_quote.quote.quote_amount_decimal();

            if quote_amount < MIN_SWAP_QUOTE_AMOUNT {
                warn!("Decaying swap amount of {quote_amount} USDC is less than minimum swap amount ({MIN_SWAP_QUOTE_AMOUNT})");
                return Ok(None);
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
            params = QuoteParams { from_amount: params.from_amount / SWAP_DECAY_FACTOR, ..params };
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
        let SwapIntoTargetTokenRequest { target_amount, quote_params } = req;

        let target_token = Token::from_addr_on_chain(&quote_params.to_token, self.chain);

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

        self.execute_swaps_into_target_token(quote_params, target_token, amount_to_cover_usdc).await
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
    ) -> Result<Vec<DecayingSwapOutcome>, ExecutionClientError> {
        let target_ticker = target_token.get_ticker().unwrap_or(target_token.get_addr());

        // Get the balances of the candidate tokens to swap out of,
        // sorted by descending value
        let swap_candidates = self.get_swap_candidates(&target_token).await?;

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
        target_token: &Token,
    ) -> Result<Vec<SwapCandidate>, ExecutionClientError> {
        let candidate_tokens: Vec<Token> = get_all_tokens()
            .into_iter()
            .filter(|token| self.swap_candidate_predicate(token, target_token))
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
    fn swap_candidate_predicate(&self, token: &Token, target_token: &Token) -> bool {
        let token_on_chain = token.get_chain() == self.chain;
        let token_not_target = token.get_addr() != target_token.get_addr();
        let token_not_stablecoin = !token.is_stablecoin();
        let token_not_usd_mock = token.get_addr() != Address::ZERO.to_string();

        token_on_chain && token_not_target && token_not_stablecoin && token_not_usd_mock
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

    // ----------------------------
    // | General Swapping Helpers |
    // ----------------------------

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
        // Fetch all quotes in parallel
        let quote_futures = self.venues.get_all_venues().into_iter().map(|venue| {
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

            if maybe_best_quote.is_none() {
                maybe_best_quote = Some(quote);
                continue;
            }

            let best_quote = maybe_best_quote.as_ref().unwrap();

            let quote_price = quote.quote.get_price(None /* buy_amount */);

            let is_sell = quote.quote.is_sell();
            info!("{venue_specifier} quote price: {quote_price} (is_sell: {is_sell})");

            let best_quote_price = best_quote.quote.get_price(None /* buy_amount */);

            let is_better_sell = is_sell && quote_price > best_quote_price;
            let is_better_buy = !is_sell && quote_price < best_quote_price;

            if is_better_sell || is_better_buy {
                maybe_best_quote = Some(quote);
            }
        }

        if let Some(ref best_quote) = maybe_best_quote {
            self.validate_quote(best_quote).await?;
        }

        Ok(maybe_best_quote)
    }

    /// Validate a quote against the Renegade price
    async fn validate_quote(
        &self,
        executable_quote: &ExecutableQuote,
    ) -> Result<(), ExecutionClientError> {
        let quote = &executable_quote.quote;

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

        if deviation > MAX_PRICE_DEVIATION {
            return Err(ExecutionClientError::quote_validation(format!(
                "Price deviation of {deviation} is greater than max price deviation of {MAX_PRICE_DEVIATION}"
            )));
        }

        Ok(())
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
        }
    }
}
