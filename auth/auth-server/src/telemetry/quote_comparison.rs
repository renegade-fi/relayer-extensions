use renegade_circuit_types::order::OrderSide;
use renegade_common::types::{token::Token, TimestampedPrice};
use std::sync::Arc;

use super::{
    helpers::{
        calculate_price_diff_bps, extend_labels_with_base_asset, record_comparison,
        reverse_decimal_correction,
    },
    labels::SIDE_TAG,
    sources::QuoteSource,
};
use renegade_api::http::external_match::ExternalQuoteResponse;

/// Represents a single quote comparison between quotes from different sources
pub struct QuoteComparison {
    pub our_price: f64,
    pub source_price: f64,
    pub source_name: String,
    pub price_diff_bips: i32,
}

/// Records metrics comparing quotes from different sources
pub struct QuoteComparisonHandler {
    sources: Vec<Box<dyn QuoteSource>>,
}

impl QuoteComparisonHandler {
    /// Create a new QuoteComparisonHandler with the given sources
    pub fn new(sources: Vec<Box<dyn QuoteSource>>) -> Self {
        Self { sources }
    }

    /// Records metrics comparing quotes from different sources
    pub async fn record_quote_comparison(
        &self,
        quote_resp: &ExternalQuoteResponse,
        extra_labels: &[(String, String)],
    ) {
        let base_token = Token::from_addr_biguint(&quote_resp.signed_quote.quote.order.base_mint);
        let quote_token = Token::from_addr_biguint(&quote_resp.signed_quote.quote.order.quote_mint);

        let ts_price: TimestampedPrice = quote_resp.signed_quote.quote.price.clone().into();
        let our_price = reverse_decimal_correction(ts_price.price, &base_token, &quote_token)
            .expect("Price correction should not fail");

        let is_sell = quote_resp.signed_quote.quote.order.side == OrderSide::Sell;
        let side_label = if is_sell { "sell" } else { "buy" };

        let mut labels = vec![(SIDE_TAG.to_string(), side_label.to_string())];
        labels.extend(extra_labels.iter().cloned());
        labels = extend_labels_with_base_asset(&base_token.get_addr(), labels);

        let amount = if is_sell {
            quote_resp.signed_quote.quote.order.base_amount
        } else {
            quote_resp.signed_quote.quote.order.quote_amount
        };

        // Compare with each source
        for source in &self.sources {
            let other_quote = source
                .get_quote(
                    base_token.clone(),
                    quote_token.clone(),
                    quote_resp.signed_quote.quote.order.side,
                    amount,
                    our_price,
                )
                .await;

            let price_diff_bips = calculate_price_diff_bps(our_price, other_quote.price, is_sell);
            let comparison = QuoteComparison {
                our_price,
                source_price: other_quote.price,
                source_name: source.name().to_string(),
                price_diff_bips,
            };

            record_comparison(&comparison, &labels);
        }
    }
}
