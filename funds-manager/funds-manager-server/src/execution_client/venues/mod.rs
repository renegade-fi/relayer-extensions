//! Venue-specific logic for getting quotes and executing swaps

use alloy_primitives::{TxHash, U256};
use async_trait::async_trait;
use funds_manager_api::quoters::{QuoteParams, SupportedExecutionVenue};

use crate::execution_client::{
    error::ExecutionClientError,
    venues::{
        bebop::BebopClient,
        cowswap::CowswapClient,
        lifi::LifiClient,
        okx::OkxClient,
        quote::{CrossVenueQuoteSource, ExecutableQuote},
    },
};

pub mod bebop;
pub mod cowswap;
pub mod lifi;
pub mod okx;
pub mod quote;

/// A collection of all execution venues used by the execution client
#[derive(Clone)]
pub struct AllExecutionVenues {
    /// The Lifi client
    pub lifi: LifiClient,
    /// The Cowswap client
    pub cowswap: CowswapClient,
    /// The Bebop client
    pub bebop: BebopClient,
    /// The Okx client. `None` if OKX startup failed (e.g. credentials rejected);
    /// the venue is then skipped instead of crashing the server.
    pub okx: Option<OkxClient>,
}

impl AllExecutionVenues {
    /// Get all venues
    pub fn get_all_venues(&self) -> Vec<&dyn ExecutionVenue> {
        // TEMP: We are disabling Cowswap by default until we have a mechanism
        // for self-trade prevention
        let mut venues: Vec<&dyn ExecutionVenue> = vec![&self.lifi, &self.bebop];
        if let Some(okx) = &self.okx {
            venues.push(okx);
        }
        venues
    }

    /// Get a venue by its specifier. Returns `None` for OKX when OKX startup
    /// failed and the venue was skipped.
    pub fn get_venue(&self, venue: SupportedExecutionVenue) -> Option<&dyn ExecutionVenue> {
        match venue {
            SupportedExecutionVenue::Lifi => Some(&self.lifi),
            SupportedExecutionVenue::Cowswap => Some(&self.cowswap),
            SupportedExecutionVenue::Bebop => Some(&self.bebop),
            SupportedExecutionVenue::Okx => self.okx.as_ref().map(|c| c as &dyn ExecutionVenue),
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
/// getting & executing quotes
#[async_trait]
pub trait ExecutionVenue: Sync {
    /// Get the name of the venue
    fn venue_specifier(&self) -> SupportedExecutionVenue;

    /// Get quotes from the venue, excluding those from the given list of
    /// sources.
    ///
    /// Each quote should represent a unique variant of `CrossVenueQuoteSource`.
    async fn get_quotes(
        &self,
        params: QuoteParams,
        excluded_quote_sources: &[CrossVenueQuoteSource],
    ) -> Result<Vec<ExecutableQuote>, ExecutionClientError>;

    /// Execute a quote from the venue
    async fn execute_quote(
        &self,
        executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError>;
}
