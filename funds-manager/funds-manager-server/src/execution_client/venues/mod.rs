//! Venue-specific logic for getting quotes and executing swaps

use alloy_primitives::U256;
use renegade_common::types::{chain::Chain, token::Token};

use crate::execution_client::venues::lifi::LifiQuoteExecutionData;

pub mod lifi;

/// An enum used to specify supported execution venues
pub enum SupportedExecutionVenue {
    /// The Lifi venue
    Lifi,
    /// The Cowswap venue
    Cowswap,
}

/// The basic information included in an execution quote,
/// agnostic of the venue that provided the quote
pub struct ExecutionQuote {
    /// The token being sold
    pub sell_token: Token,
    /// The token being bought
    pub buy_token: Token,
    /// The amount of the token being sold, in atoms
    pub sell_amount: U256,
    /// The quoted amount of the token being bought, in atoms
    pub buy_amount: U256,
    /// The venue that provided the quote
    pub venue: SupportedExecutionVenue,
    /// The chain for which the quote was generated
    pub chain: Chain,
}

/// An enum wrapping the venue-specific auxiliary data needed to execute a quote
pub enum QuoteExecutionData {
    /// Lifi-specific quote execution data
    Lifi(LifiQuoteExecutionData),
    /// Cowswap-specific quote execution data
    // TODO: Implement Cowswap quote execution data
    Cowswap(),
}

/// An executable quote, which includes the basic quote information
/// along with any auxiliary data needed to execute the quote
pub struct ExecutableQuote {
    /// The quote
    pub quote: ExecutionQuote,
    /// The venue-specific auxiliary data needed to execute the quote
    pub execution_data: QuoteExecutionData,
}
