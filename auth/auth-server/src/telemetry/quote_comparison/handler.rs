use futures_util::future::join_all;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

use renegade_api::http::external_match::AtomicMatchApiBundle;

use crate::telemetry::{
    helpers::record_comparison,
    sources::{QuoteResponse, QuoteSource},
};

use super::QuoteComparison;

/// Records metrics comparing quotes from different sources
pub struct QuoteComparisonHandler {
    sources: Vec<QuoteSource>,
}

impl QuoteComparisonHandler {
    /// Create a new QuoteComparisonHandler with the given sources
    pub fn new(sources: Vec<QuoteSource>) -> Self {
        Self { sources }
    }

    /// Records a comparison for a single source
    async fn record_comparison_for_source(
        source: QuoteSource,
        our_quote: &QuoteResponse,
        base_token: Token,
        quote_token: Token,
        side: OrderSide,
        amount: u128,
        labels: Vec<(String, String)>,
    ) {
        let quote = source.get_quote(base_token, quote_token, side, amount).await;
        let comparison =
            QuoteComparison { our_quote, source_quote: &quote, source_name: source.name() };

        record_comparison(&comparison, side, &labels);
    }

    /// Records metrics comparing quotes from different sources
    pub async fn record_quote_comparison(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        extra_labels: &[(String, String)],
    ) {
        let base_token = Token::from_addr(&match_bundle.match_result.base_mint);
        let quote_token = Token::from_addr(&match_bundle.match_result.quote_mint);

        let our_quote: QuoteResponse = match_bundle.into();

        let amount = if match_bundle.match_result.direction == OrderSide::Sell {
            match_bundle.match_result.base_amount
        } else {
            match_bundle.match_result.quote_amount
        };

        let mut futures = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            futures.push(Self::record_comparison_for_source(
                source.clone(),
                &our_quote,
                base_token.clone(),
                quote_token.clone(),
                match_bundle.match_result.direction,
                amount,
                extra_labels.to_vec(),
            ));
        }

        // Execute all futures concurrently and wait for them to complete
        join_all(futures).await;
    }
}
