//! Handlers for executing swaps

use std::cmp::Ordering;

use alloy::{
    eips::BlockId,
    hex,
    network::TransactionBuilder,
    providers::{DynProvider, Provider},
    rpc::types::{TransactionReceipt, TransactionRequest},
    signers::local::PrivateKeySigner,
};
use alloy_primitives::{Address, Log, U256};
use alloy_sol_types::SolEvent;
use funds_manager_api::{
    quoters::{LiFiQuoteParams, SwapIntoTargetTokenRequest},
    u256_try_into_u64,
};
use renegade_common::types::token::{get_all_tokens, Token};
use tracing::{info, instrument, warn};

use crate::{
    execution_client::venues::quote::{ExecutableQuote, ExecutionQuote},
    helpers::IERC20::{self, Transfer},
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
const MAX_PRICE_DEVIATION: f64 = 0.02; // 1%
/// The amount to increase an approval by for a swap
///
/// We "over-approve" so that we don't need to re-approve on every swap
const APPROVAL_AMPLIFIER: U256 = U256::from_limbs([4, 0, 0, 0]);
/// The address of the LiFi diamond (same address on Arbitrum One and Base
/// Mainnet), constantized here to simplify approvals
const LIFI_DIAMOND_ADDRESS: Address =
    Address::new(hex!("0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae"));
/// The buffer to scale the target amount by when executing swaps to cover it,
/// to account for price drift
const SWAP_TO_COVER_BUFFER: f64 = 1.1;

// ---------
// | Types |
// ---------

/// The outcome of an executed swap
pub struct SwapOutcome {
    /// The quote that was executed
    pub quote: ExecutionQuote,
    /// The actual amount of the token that was bought
    pub buy_amount_actual: U256,
    /// The transaction receipt of the swap
    pub receipt: TransactionReceipt,
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

// -----------
// | Helpers |
// -----------

/// Compute the gas cost of a transaction in WEI
fn get_gas_cost(receipt: &TransactionReceipt) -> U256 {
    U256::from(receipt.gas_used) * U256::from(receipt.effective_gas_price)
}

// --------------------
// | Execution Client |
// --------------------

impl ExecutionClient {
    /// Construct a swap transaction from an execution quote
    async fn build_swap_tx(
        &self,
        quote: &ExecutableQuote,
        client: &DynProvider,
    ) -> Result<TransactionRequest, ExecutionClientError> {
        let lifi_execution_data = quote.execution_data.lifi()?;

        let latest_block = client
            .get_block(BlockId::latest())
            .await
            .map_err(ExecutionClientError::onchain)?
            .ok_or(ExecutionClientError::onchain("No latest block found"))?;

        let latest_basefee = latest_block
            .header
            .base_fee_per_gas
            .ok_or(ExecutionClientError::onchain("No basefee found"))?
            as u128;

        let gas_limit = u256_try_into_u64(lifi_execution_data.gas_limit)
            .map_err(ExecutionClientError::onchain)?;

        let tx = TransactionRequest::default()
            .with_to(lifi_execution_data.to)
            .with_from(lifi_execution_data.from)
            .with_value(lifi_execution_data.value)
            .with_input(lifi_execution_data.data.clone())
            .with_max_fee_per_gas(latest_basefee * 2)
            .with_max_priority_fee_per_gas(latest_basefee * 2)
            .with_gas_limit(gas_limit);

        Ok(tx)
    }

    /// Attempt to execute a swap, retrying failed swaps with
    /// decreased quotes down to a minimum trade size.
    ///
    /// Returns the quote, transaction receipt, and cumulative gas cost of all
    /// attempted swaps
    pub async fn swap_immediate_decaying(
        &self,
        mut params: LiFiQuoteParams,
        wallet: PrivateKeySigner,
    ) -> Result<Option<SwapOutcome>, ExecutionClientError> {
        // Approve the top-level sell amount
        let sell_token_amount = params.from_amount;
        let sell_token_address: Address =
            params.from_token.parse().map_err(ExecutionClientError::parse)?;

        self.approve_erc20_allowance(
            sell_token_address,
            LIFI_DIAMOND_ADDRESS,
            sell_token_amount,
            &wallet,
        )
        .await?;

        let mut cumulative_gas_cost = U256::ZERO;
        loop {
            let augmented_quote = match self.get_augmented_quote(params.clone(), self.chain).await?
            {
                None => return Ok(None),
                Some(augmented_quote) => augmented_quote,
            };

            // Submit the swap
            let client = self.get_signing_provider(wallet.clone());
            let tx = self.build_swap_tx(&executable_quote, &client).await?;
            let receipt = self.send_tx(tx, &client).await?;
            cumulative_gas_cost += get_gas_cost(&receipt);

            // If the swap succeeds, return
            if receipt.status() {
                let buy_amount_actual = self.get_buy_amount_actual(&receipt, &executable_quote)?;

                return Ok(Some(SwapOutcome {
                    quote: executable_quote.quote,
                    buy_amount_actual,
                    receipt,
                    cumulative_gas_cost,
                }));
            }

            // Otherwise, decrease the swap size and try again
            warn!("tx ({:#x}) failed, retrying w/ reduced-size quote", receipt.transaction_hash);
            params =
                LiFiQuoteParams { from_amount: params.from_amount / SWAP_DECAY_FACTOR, ..params };
        }
    }

    /// Try to execute swaps to cover a target amount of a token,
    /// first checking if any swaps are necessary.
    ///
    /// Returns a vector of outcomes for the executed swaps.
    pub async fn try_swap_into_target_token(
        &self,
        req: SwapIntoTargetTokenRequest,
        wallet: PrivateKeySigner,
    ) -> Result<Vec<SwapOutcome>, ExecutionClientError> {
        let SwapIntoTargetTokenRequest { target_amount, quote_params } = req;

        let target_token = Token::from_addr_on_chain(&quote_params.to_token, self.chain);

        // Check that the current token balances doesn't already cover the target amount
        let current_balance =
            self.get_erc20_balance(&target_token.addr, &wallet.address().to_string()).await?;

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
            target_token,
            amount_to_cover_usdc,
            quote_params,
            wallet,
        )
        .await
    }

    // ---------------------------------
    // | Target Token Swapping Helpers |
    // ---------------------------------

    /// Execute swaps to cover a target amount of a token.
    ///
    /// Returns a vector of outcomes for the executed swaps.
    async fn execute_swaps_into_target_token(
        &self,
        target_token: Token,
        amount_to_cover_usdc: f64,
        params: LiFiQuoteParams,
        wallet: PrivateKeySigner,
    ) -> Result<Vec<SwapOutcome>, ExecutionClientError> {
        let target_ticker = target_token.get_ticker().unwrap_or(target_token.get_addr());

        // Get the balances of the candidate tokens to swap out of,
        // sorted by descending value
        let swap_candidates =
            self.get_swap_candidates(&target_token, &wallet.address().to_string()).await?;

        // We increase the amount to cover by a fixed buffer to account for drift
        // in the prices sampled when getting swap candidates
        let mut remaining_amount_usdc = amount_to_cover_usdc * SWAP_TO_COVER_BUFFER;
        info!("Need to cover ${amount_to_cover_usdc} {target_ticker}, purchasing ${remaining_amount_usdc}");

        let mut outcomes = vec![];
        for candidate in swap_candidates {
            let token = &candidate.token;
            let ticker = token.get_ticker().unwrap_or(token.get_addr());

            let maybe_outcome = self
                .try_swap_candidate(
                    target_token.get_addr(),
                    candidate,
                    remaining_amount_usdc,
                    params.clone(),
                    wallet.clone(),
                )
                .await?;

            if maybe_outcome.is_none() {
                break;
            }

            let swap_outcome = maybe_outcome.unwrap();
            let sell_amount = swap_outcome.quote.sell_amount_decimal();
            let quoted_buy_amount = swap_outcome.quote.buy_amount_decimal();

            outcomes.push(swap_outcome);

            info!("Swapped {sell_amount} {ticker} for {quoted_buy_amount} {target_ticker}");

            remaining_amount_usdc -= quoted_buy_amount;
        }

        Ok(outcomes)
    }

    /// Get the candidate token balances to swap out of to cover some amount of
    /// the target token. Returns a vector of (token, balance, price)
    /// tuples, sorted by descending value.
    async fn get_swap_candidates(
        &self,
        target_token: &Token,
        wallet_address: &str,
    ) -> Result<Vec<SwapCandidate>, ExecutionClientError> {
        let candidate_tokens: Vec<Token> = get_all_tokens()
            .into_iter()
            .filter(|token| self.swap_candidate_predicate(token, target_token))
            .collect();

        let mut swap_candidates = vec![];
        for token in candidate_tokens {
            let balance = self.get_erc20_balance(&token.addr, wallet_address).await?;
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
        target_token_addr: String,
        candidate: SwapCandidate,
        amount_to_cover_usdc: f64,
        params: LiFiQuoteParams,
        wallet: PrivateKeySigner,
    ) -> Result<Option<SwapOutcome>, ExecutionClientError> {
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
        let swap_params = LiFiQuoteParams {
            to_token: target_token_addr,
            from_token: token.get_addr(),
            from_amount: U256::from(swap_amount),
            ..params
        };

        let swap_outcome = self.swap_immediate_decaying(swap_params, wallet).await?;
        Ok(swap_outcome)
    }

    // ----------------------------
    // | General Swapping Helpers |
    // ----------------------------

    /// Get an execution quote for a swap
    #[instrument(skip_all)]
    pub async fn get_executable_quote(
        &self,
        params: LiFiQuoteParams,
        chain: Chain,
    ) -> Result<Option<AugmentedExecutionQuote>, ExecutionClientError> {
        // Zero quotes may be requested when executing a decaying swap,
        // in which case no quote is possible
        if params.from_amount == U256::ZERO {
            return Ok(None);
        }

        let quote = self.get_quote(params).await?;
        let augmented_quote = AugmentedExecutionQuote::new(quote.clone(), chain);
        self.validate_quote(&augmented_quote).await?;

        let quote_amount = executable_quote.quote.quote_amount_decimal();

        if quote_amount < MIN_SWAP_QUOTE_AMOUNT {
            warn!("Recursive swap amount of {quote_amount} USDC is less than minimum swap amount ({MIN_SWAP_QUOTE_AMOUNT})");
            return Ok(None);
        }

        Ok(Some(executable_quote))
    }

    /// Validate a quote against the Renegade price
    async fn validate_quote(
        &self,
        executable_quote: &ExecutableQuote,
    ) -> Result<(), ExecutionClientError> {
        // Get the renegade price for the pair
        let base_addr = &augmented_quote.get_base_token().addr;
        let renegade_price =
            self.price_reporter.get_price(base_addr, augmented_quote.chain).await?;
        let quote_price = augmented_quote
            .get_decimal_corrected_price()
            .map_err(ExecutionClientError::quote_validation)?;

        // Check that the price is within the max price impact
        let deviation = if augmented_quote.is_buy() {
            (quote_price - renegade_price) / renegade_price
        } else {
            (renegade_price - quote_price) / renegade_price
        };

        if deviation > MAX_PRICE_DEVIATION {
            let err_msg = format!(
                "Price deviation of {deviation} is greater than max price deviation of {MAX_PRICE_DEVIATION}; Base addr: {base_addr}; Renegade price: {renegade_price}; Quote price: {quote_price}"
            );
            return Err(ExecutionClientError::quote_validation(err_msg));
        }

        Ok(())
    }

    /// Approve an erc20 allowance
    #[instrument(skip(self, wallet))]
    pub(crate) async fn approve_erc20_allowance(
        &self,
        token_address: Address,
        spender: Address,
        amount: U256,
        wallet: &PrivateKeySigner,
    ) -> Result<(), ExecutionClientError> {
        let client = self.get_signing_provider(wallet.clone());
        let erc20 = IERC20::new(token_address, client);

        // First, check if the allowance is already sufficient
        let allowance = erc20
            .allowance(wallet.address(), spender)
            .call()
            .await
            .map_err(ExecutionClientError::onchain)?;

        if allowance >= amount {
            info!("Already approved erc20 allowance for {spender:#x}");
            return Ok(());
        }

        // Otherwise, approve the allowance
        let approval_amount = amount * APPROVAL_AMPLIFIER;
        let tx = erc20.approve(spender, approval_amount);
        let pending_tx = tx.send().await.map_err(ExecutionClientError::onchain)?;

        let receipt = pending_tx.get_receipt().await.map_err(ExecutionClientError::onchain)?;

        info!("Approved erc20 allowance at: {:#x}", receipt.transaction_hash);
        Ok(())
    }

    /// Extract the transfer amount from a transaction receipt
    fn get_buy_amount_actual(
        &self,
        receipt: &TransactionReceipt,
        executable_quote: &ExecutableQuote,
    ) -> Result<U256, ExecutionClientError> {
        let buy_mint = executable_quote.quote.buy_token.get_alloy_address();
        let recipient = executable_quote.execution_data.lifi()?.from;

        let logs: Vec<Log<Transfer>> = receipt
            .logs()
            .iter()
            .filter_map(|log| {
                if log.address() != buy_mint {
                    None
                } else {
                    Transfer::decode_log(&log.inner).ok()
                }
            })
            .collect();

        logs.iter()
            .find_map(|transfer| if transfer.to == recipient { Some(transfer.value) } else { None })
            .ok_or(ExecutionClientError::onchain("no matching transfer event found"))
    }
}
