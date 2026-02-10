//! Logic & helpers for swapping into target tokens

use std::{cmp::Ordering, iter};

use alloy_primitives::{Address, U256};
use funds_manager_api::{
    quoters::{QuoteParams, SwapIntoTargetTokenRequest},
    u256_try_into_u128,
};
use renegade_types_core::{Token, USD_TICKER, USDC_TICKER, get_all_tokens};
use tracing::{info, warn};

use crate::execution_client::{
    ExecutionClient,
    error::ExecutionClientError,
    swap::{DecayingSwapOutcome, MIN_SWAP_QUOTE_AMOUNT},
};

// -------------
// | Constants |
// -------------

/// The buffer to scale the target amount by when executing swaps to cover it,
/// to account for price drift
const SWAP_TO_COVER_BUFFER: f64 = 1.1;

// ---------
// | Types |
// ---------

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

impl ExecutionClient {
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
            info!(
                "Current {ticker} balance ({current_balance}) is greater than target amount ({target_amount}), skipping swaps"
            );
            return Ok(vec![]);
        }

        let amount_to_cover = target_amount - current_balance;
        let price = self.price_reporter.get_price(&target_token.addr, self.chain).await?;
        let amount_to_cover_usdc = amount_to_cover * price;

        // Check that the amount to cover is greater than the minimum swap amount
        if amount_to_cover_usdc < MIN_SWAP_QUOTE_AMOUNT {
            info!(
                "Target token value to cover (${amount_to_cover_usdc}) is less than minimum swap amount (${MIN_SWAP_QUOTE_AMOUNT}), skipping swaps"
            );
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
        info!(
            "Need to cover ${amount_to_cover_usdc} {target_ticker}, purchasing ${remaining_amount_usdc}"
        );

        let mut outcomes = vec![];
        for candidate in swap_candidates {
            if remaining_amount_usdc < MIN_SWAP_QUOTE_AMOUNT {
                info!(
                    "Remaining amount to cover (${remaining_amount_usdc}) is less than minimum swap amount (${MIN_SWAP_QUOTE_AMOUNT}), stopping swaps"
                );
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
        let token_not_dummy_usd =
            token.get_ticker().map(|ticker| ticker != USD_TICKER).unwrap_or(true);

        token_on_chain && token_not_excluded && token_not_dummy_usd
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

    /// For each token, get the USDC value of the purchase amount needed to meet
    /// the desired balance
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

        if usdc_target_balance == 0.0 {
            info!("No purchases needed, skipping swap into USDC");
            return Ok(vec![]);
        }

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
}
