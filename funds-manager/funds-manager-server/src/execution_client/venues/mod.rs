//! Venue-specific logic for getting quotes and executing swaps

use std::fmt::Display;

use alloy_primitives::{TxHash, U256};
use async_trait::async_trait;
use funds_manager_api::quoters::QuoteParams;

use crate::execution_client::{
    error::ExecutionClientError,
    venues::{cowswap::CowswapClient, lifi::LifiClient, quote::ExecutableQuote},
};

pub mod cowswap;
pub mod lifi;
pub mod quote;

/// An enum used to specify supported execution venues
#[derive(Debug)]
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

/// A collection of all execution venues used by the execution client
#[derive(Clone)]
pub struct AllExecutionVenues {
    /// The Lifi client
    pub lifi: LifiClient,
    /// The Cowswap client
    pub cowswap: CowswapClient,
}

impl AllExecutionVenues {
    /// Get all venues
    pub fn get_all_venues(&self) -> Vec<&dyn ExecutionVenue> {
        vec![&self.lifi, &self.cowswap]
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
/// getting & executing quotes
#[async_trait]
pub trait ExecutionVenue: Sync {
    /// Get the name of the venue
    fn venue_specifier(&self) -> SupportedExecutionVenue;

    /// Get a quote from the venue
    async fn get_quote(&self, params: QuoteParams)
        -> Result<ExecutableQuote, ExecutionClientError>;

    /// Execute a quote from the venue
    async fn execute_quote(
        &self,
        executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError>;
}
