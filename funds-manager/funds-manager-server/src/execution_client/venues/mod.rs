//! Venue-specific logic for getting quotes and executing swaps

use std::fmt::Display;

use alloy_primitives::{TxHash, U256};
use async_trait::async_trait;
use funds_manager_api::quoters::QuoteParams;

use crate::execution_client::{
    error::ExecutionClientError,
    venues::quote::{ExecutableQuote, ExecutionQuote},
};

pub mod lifi;
pub mod quote;

/// An enum used to specify supported execution venues
pub enum SupportedExecutionVenue {
    /// The Lifi venue
    Lifi,
    /// The Cowswap venue
    Cowswap,
}

impl Display for SupportedExecutionVenue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedExecutionVenue::Lifi => write!(f, "Lifi"),
            SupportedExecutionVenue::Cowswap => write!(f, "Cowswap"),
        }
    }
}

/// The outcome of an executed swap
pub struct ExecutionResult {
    /// The actual amount of the token that was bought
    pub buy_amount_actual: U256,
    /// The amount of gas spent executing the quote
    pub gas_cost: U256,
    /// The transaction hash in which the swap was executed,
    /// if the swap was successful
    pub tx_hash: Option<TxHash>,
}

/// Exposes the basic functionality of an execution venue:
/// Getting & executing quotes
#[async_trait]
pub trait ExecutionVenue {
    /// The venue-specific auxiliary data required to execute a quote
    type ExecutionData;

    /// Get a quote from the venue
    async fn get_quote(&self, params: QuoteParams)
        -> Result<ExecutableQuote, ExecutionClientError>;

    /// Execute a quote from the venue
    async fn execute_quote(
        &self,
        quote: &ExecutionQuote,
        execution_data: &Self::ExecutionData,
    ) -> Result<ExecutionResult, ExecutionClientError>;
}
