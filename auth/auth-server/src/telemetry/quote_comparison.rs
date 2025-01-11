use futures_util::future::join_all;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

use super::{
    helpers::{
        calculate_implied_price, calculate_price_diff_bps, extend_labels_with_base_asset,
        record_comparison,
    },
    labels::SIDE_TAG,
    sources::QuoteSource,
};
use renegade_api::http::external_match::AtomicMatchApiBundle;

/// Represents a single quote comparison between quotes from different sources
pub struct QuoteComparison {
    pub our_price: f64,
    pub source_price: f64,
    pub source_name: String,
    pub price_diff_bips: i32,
}

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
        our_price: f64,
        base_token: Token,
        quote_token: Token,
        side: OrderSide,
        amount: u128,
        labels: Vec<(String, String)>,
    ) {
        let quote = source.get_quote(base_token, quote_token, side, amount).await;
        let price_diff_bips = calculate_price_diff_bps(our_price, quote.price, side);
        let comparison = QuoteComparison {
            our_price,
            source_price: quote.price,
            source_name: source.name().to_string(),
            price_diff_bips,
        };
        record_comparison(&comparison, &labels);
    }

    /// Records metrics comparing quotes from different sources
    pub async fn record_quote_comparison(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        extra_labels: &[(String, String)],
    ) {
        let base_token = Token::from_addr(&match_bundle.match_result.base_mint);
        let quote_token = Token::from_addr(&match_bundle.match_result.quote_mint);

        let our_price = calculate_implied_price(match_bundle, false)
            .expect("Price calculation should not fail");

        let is_sell = match_bundle.match_result.direction == OrderSide::Sell;
        let side_label = if is_sell { "sell" } else { "buy" };

        let mut labels = vec![(SIDE_TAG.to_string(), side_label.to_string())];
        labels.extend(extra_labels.iter().cloned());
        labels = extend_labels_with_base_asset(&base_token.get_addr(), labels);

        let amount = if is_sell {
            match_bundle.match_result.base_amount
        } else {
            match_bundle.match_result.quote_amount
        };

        let mut futures = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            futures.push(Self::record_comparison_for_source(
                source.clone(),
                our_price,
                base_token.clone(),
                quote_token.clone(),
                match_bundle.match_result.direction,
                amount,
                labels.clone(),
            ));
        }

        // Execute all futures concurrently and wait for them to complete
        join_all(futures).await;
    }
}
