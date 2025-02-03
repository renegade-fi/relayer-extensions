//! Defines the quote comparison handler

use ethers::utils::format_units;
use ethers::{providers::Middleware, types::U256};
use futures_util::future::join_all;
use renegade_arbitrum_client::client::ArbitrumClient;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;

use renegade_api::http::external_match::AtomicMatchApiBundle;

use crate::{
    error::AuthServerError,
    telemetry::{
        helpers::{
            record_net_output_value_comparison, record_output_value_net_of_fee_comparison,
            record_output_value_net_of_gas_comparison, record_quote_price_comparison,
        },
        sources::{QuoteResponse, QuoteSource},
    },
};

use super::{price_reporter_client::PriceReporterClient, QuoteComparison};

/// Records metrics comparing quotes from different sources
pub struct QuoteComparisonHandler {
    /// The sources to compare quotes from
    sources: Vec<QuoteSource>,
    /// The arbitrum client
    arbitrum_client: ArbitrumClient,
    /// The price reporter client
    price_reporter_client: PriceReporterClient,
}

impl QuoteComparisonHandler {
    /// Create a new QuoteComparisonHandler with the given sources
    pub fn new(
        sources: Vec<QuoteSource>,
        arbitrum_client: ArbitrumClient,
        price_reporter_client: PriceReporterClient,
    ) -> Self {
        Self { sources, arbitrum_client, price_reporter_client }
    }

    /// Records metrics comparing quotes from different sources
    pub async fn record_quote_comparison(
        &self,
        match_bundle: &AtomicMatchApiBundle,
        extra_labels: &[(String, String)],
    ) {
        let our_quote: QuoteResponse = match_bundle.into();

        let amount = if match_bundle.match_result.direction == OrderSide::Sell {
            match_bundle.match_result.base_amount
        } else {
            match_bundle.match_result.quote_amount
        };

        let mut futures = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            futures.push(self.record_comparison_for_source(
                source.clone(),
                &our_quote,
                match_bundle.match_result.direction,
                amount,
                extra_labels.to_vec(),
            ));
        }

        // Execute all futures concurrently and wait for them to complete
        join_all(futures).await;
    }

    /// Records a comparison for a single source
    async fn record_comparison_for_source(
        &self,
        source: QuoteSource,
        our_quote: &QuoteResponse,
        side: OrderSide,
        amount: u128,
        labels: Vec<(String, String)>,
    ) -> Result<(), AuthServerError> {
        let base_token = Token::from_addr(&our_quote.base_mint);
        let quote_token = Token::from_addr(&our_quote.quote_mint);

        let quote = source.get_quote(base_token, quote_token, side, amount).await;
        let usdc_per_gas = self.get_usdc_per_gas().await?;
        let comparison = QuoteComparison { our_quote, source_quote: &quote, usdc_per_gas };

        record_quote_price_comparison(&comparison, side, &labels);
        record_output_value_net_of_gas_comparison(&comparison, side, &labels);
        record_output_value_net_of_fee_comparison(&comparison, side, &labels);
        record_net_output_value_comparison(&comparison, side, &labels);
        Ok(())
    }
}

impl QuoteComparisonHandler {
    /// Calculates the USDC cost per unit of gas
    async fn get_usdc_per_gas(&self) -> Result<f64, AuthServerError> {
        let gas_price_eth = self.fetch_gas_price_eth().await?;

        let usdc_per_eth =
            self.price_reporter_client.get_eth_price().await.map_err(AuthServerError::custom)?;

        Ok(usdc_per_eth * gas_price_eth)
    }

    /// Fetches the current gas price and converts it to ETH
    async fn fetch_gas_price_eth(&self) -> Result<f64, AuthServerError> {
        // Fetch gas price in wei
        let gas_price: U256 = self
            .arbitrum_client
            .client()
            .get_gas_price()
            .await
            .map_err(|e| AuthServerError::Custom(e.to_string()))?;

        // Convert wei to eth
        let eth_string =
            format_units(gas_price, "eth").map_err(|e| AuthServerError::Custom(e.to_string()))?;

        eth_string.parse::<f64>().map_err(|e| AuthServerError::Custom(e.to_string()))
    }
}
