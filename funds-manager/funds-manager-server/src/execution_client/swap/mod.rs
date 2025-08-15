//! Handlers for executing swaps

use alloy_primitives::{TxHash, U256};

use crate::execution_client::venues::quote::ExecutionQuote;

pub mod swap_immediate;
pub mod swap_to_target;

// -------------
// | Constants |
// -------------

/// The minimum amount of USDC that will be attempted to be swapped recursively
pub(crate) const MIN_SWAP_QUOTE_AMOUNT: f64 = 10.0; // 10 USDC
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
