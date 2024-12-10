//! Helper methods for capturing telemetry information throughout the auth
//! server

use crate::error::AuthServerError;
use crate::telemetry::labels::{
    ASSET_METRIC_TAG, EXTERNAL_MATCH_BASE_VOLUME, EXTERNAL_MATCH_QUOTE_VOLUME,
    EXTERNAL_ORDER_BASE_VOLUME, EXTERNAL_ORDER_QUOTE_VOLUME, KEY_DESCRIPTION_METRIC_TAG,
    NUM_EXTERNAL_MATCH_REQUESTS,
};
use renegade_api::http::external_match::{
    AtomicMatchApiBundle, ExternalMatchRequest, ExternalMatchResponse,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_circuit_types::order::OrderSide;
use renegade_common::types::token::Token;
use renegade_util::hex::biguint_to_hex_addr;
use tracing::warn;

/// Get the human-readable asset and volume of
/// the given mint and amount.
/// The asset is the token ticker, if it is found, otherwise
/// the token's address.
/// The amount is the decimal amount of the transfer, going through
/// lossy f64 conversion via the associated number of decimals
fn get_asset_and_volume(mint: &str, amount: u128) -> (String, f64) {
    let token = Token::from_addr(mint);
    let asset = token.get_ticker().unwrap_or(mint.to_string());
    let volume = token.convert_to_decimal(amount);

    (asset, volume)
}

/// Calculates the quote per base price from a match bundle
/// Returns the price as an f64 decimal adjusted value
fn calculate_implied_price(match_bundle: &AtomicMatchApiBundle) -> f64 {
    let (quote, base) = match match_bundle.match_result.direction {
        OrderSide::Buy => (&match_bundle.send, &match_bundle.receive),
        OrderSide::Sell => (&match_bundle.receive, &match_bundle.send),
    };

    let quote_amt = Token::from_addr(&quote.mint).convert_to_decimal(quote.amount);
    let base_amt = Token::from_addr(&base.mint).convert_to_decimal(base.amount);
    quote_amt / base_amt
}

/// Converts a decimal amount to token native units, accounting for the token's
/// decimals. This is the inverse operation of convert_to_decimal.
fn convert_from_decimal(token: &Token, decimal_amount: f64) -> u128 {
    let decimals = token.get_decimals().unwrap_or_default();
    let decimal_correction = 10f64.powi(decimals as i32);
    let corrected_amount = decimal_amount * decimal_correction;
    corrected_amount as u128
}

/// Record a volume metric with the given extra tags
fn record_volume_with_tags(
    mint: &str,
    amount: u128,
    volume_metric_name: &'static str,
    extra_labels: &[(String, String)],
) {
    let (asset, volume) = get_asset_and_volume(mint, amount);
    let mut labels = vec![(ASSET_METRIC_TAG.to_string(), asset)];
    let extra_labels = extra_labels.iter().map(|(k, v)| (k.clone(), v.clone()));
    labels.extend(extra_labels);

    // We use a gauge metric here to be able to capture a float value
    // for the volume
    metrics::gauge!(volume_metric_name, labels.as_slice()).set(volume);
}

/// Records metrics for the incoming external match request
fn record_external_match_request_metrics(
    req: &ExternalMatchRequest,
    price: f64,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    // Record external order volume
    let base_mint = biguint_to_hex_addr(&req.external_order.base_mint);
    let quote_mint = biguint_to_hex_addr(&req.external_order.quote_mint);

    // Calculate amount in base
    let fixed_point_price = FixedPoint::from_f64_round_down(price);
    let order = req.external_order.to_order_with_price(fixed_point_price);

    // Calculate amount in quote
    let (_, volume) = get_asset_and_volume(&base_mint, order.amount);
    let quote_token = Token::from_addr(&quote_mint);
    let quote_amount = convert_from_decimal(&quote_token, volume * price);

    record_volume_with_tags(&base_mint, order.amount, EXTERNAL_ORDER_BASE_VOLUME, labels);
    record_volume_with_tags(&quote_mint, quote_amount, EXTERNAL_ORDER_QUOTE_VOLUME, labels);

    Ok(())
}

/// Records metrics for the external match response (match bundle)
fn record_external_match_response_metrics(
    resp: &ExternalMatchResponse,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let (quote, base) = match resp.match_bundle.match_result.direction {
        OrderSide::Buy => (&resp.match_bundle.send, &resp.match_bundle.receive),
        OrderSide::Sell => (&resp.match_bundle.receive, &resp.match_bundle.send),
    };

    record_volume_with_tags(&quote.mint, quote.amount, EXTERNAL_MATCH_QUOTE_VOLUME, labels);
    record_volume_with_tags(&base.mint, base.amount, EXTERNAL_MATCH_BASE_VOLUME, labels);

    Ok(())
}

/// Records a counter metric for a given base mint and key description
fn record_endpoint_metrics(base_mint: &str, key_description: String, metric_name: &'static str) {
    let (asset, _) = get_asset_and_volume(base_mint, 0);
    let labels = vec![
        (ASSET_METRIC_TAG.to_string(), asset),
        (KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description),
    ];

    metrics::counter!(metric_name, &labels).increment(1);
}

/// Records all metrics related to an external match request and response
pub fn record_external_match_metrics(
    req_body: &[u8],
    resp_body: &[u8],
    key_description: String,
) -> Result<(), AuthServerError> {
    // Parse request and response
    let match_req =
        serde_json::from_slice::<ExternalMatchRequest>(req_body).map_err(AuthServerError::serde)?;
    let match_resp = serde_json::from_slice::<ExternalMatchResponse>(resp_body)
        .map_err(AuthServerError::serde)?;

    // Record atomic match request counter
    let base_mint = biguint_to_hex_addr(&match_req.external_order.base_mint);
    record_endpoint_metrics(&base_mint, key_description.clone(), NUM_EXTERNAL_MATCH_REQUESTS);

    let labels = vec![(KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description)];

    // Get price
    let price = calculate_implied_price(&match_resp.match_bundle);

    // Record request metrics
    if let Err(e) = record_external_match_request_metrics(&match_req, price, &labels) {
        warn!("Error recording request metrics: {e}");
    }

    // Record response metrics
    if let Err(e) = record_external_match_response_metrics(&match_resp, &labels) {
        warn!("Error recording response metrics: {e}");
    }

    Ok(())
}
