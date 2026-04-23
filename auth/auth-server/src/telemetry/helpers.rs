//! Helper methods for capturing telemetry information throughout the auth
//! server

use renegade_api::http::external_match::{
    AtomicMatchApiBundle, ExternalOrder, MalleableAtomicMatchApiBundle,
};
use renegade_circuit_types::{fixed_point::FixedPoint, order::OrderSide};
use renegade_common::types::token::Token;
use renegade_constants::{
    DEFAULT_EXTERNAL_MATCH_RELAYER_FEE, NATIVE_ASSET_ADDRESS, NATIVE_ASSET_WRAPPER_TICKER, Scalar,
};
use renegade_crypto::fields::scalar_to_u128;
use renegade_util::hex::{biguint_from_hex_string, biguint_to_hex_addr};
use tracing::warn;

use crate::{
    error::AuthServerError,
    telemetry::labels::{
        ASSET_METRIC_TAG, BASE_ASSET_METRIC_TAG, EXTERNAL_MATCH_BASE_VOLUME,
        EXTERNAL_MATCH_FILL_RATIO, EXTERNAL_MATCH_QUOTE_VOLUME, EXTERNAL_ORDER_BASE_VOLUME,
        EXTERNAL_ORDER_QUOTE_VOLUME, UNSUCCESSFUL_RELAYER_REQUEST_COUNT,
    },
};

use super::labels::{
    KEY_DESCRIPTION_METRIC_TAG, NUM_EXTERNAL_MATCH_REQUESTS, QUOTE_NOT_FOUND_COUNT,
    REQUEST_PATH_METRIC_TAG, SIDE_TAG,
};

/// Maximum quote volume (in decimal whole-unit terms) for which we still record
/// volume metrics. Requests above this threshold are considered outliers and
/// only the request count metric is recorded to prevent skewing aggregates.
const MAX_EXTERNAL_ORDER_QUOTE_VOLUME: f64 = 10_000_000.0;

// --- Asset and Volume Helpers --- //

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

// --- Price Calculation --- //

/// Calculates the decimal-corrected quote per base price from a match bundle
/// Returns the price as an f64 decimal adjusted value, accounting for the
/// difference in decimal places between quote and base tokens if
/// decimal_correct is true
pub(crate) fn calculate_implied_price(
    match_bundle: &AtomicMatchApiBundle,
    decimal_correct: bool,
) -> Result<f64, AuthServerError> {
    let (base, quote) = match match_bundle.match_result.direction {
        OrderSide::Buy => (&match_bundle.receive, &match_bundle.send),
        OrderSide::Sell => (&match_bundle.send, &match_bundle.receive),
    };

    let trades_native_asset =
        biguint_from_hex_string(&base.mint) == biguint_from_hex_string(NATIVE_ASSET_ADDRESS);
    let base_token = if trades_native_asset {
        Token::from_ticker(NATIVE_ASSET_WRAPPER_TICKER)
    } else {
        Token::from_addr(&base.mint)
    };

    let quote_token = Token::from_addr(&quote.mint);

    let base_decimals = base_token.get_decimals().ok_or_else(|| {
        AuthServerError::Serde(format!("No decimals for {}", base_token.get_addr()))
    })?;
    let quote_decimals = quote_token.get_decimals().ok_or_else(|| {
        AuthServerError::Serde(format!("No decimals for {}", quote_token.get_addr()))
    })?;

    let base_amt = base_token.convert_to_decimal(base.amount);
    let quote_amt = quote_token.convert_to_decimal(quote.amount);

    let uncorrected_price = quote_amt / base_amt;
    if decimal_correct {
        let decimal_diff = quote_decimals as i32 - base_decimals as i32;
        Ok(uncorrected_price * 10f64.powi(decimal_diff))
    } else {
        Ok(uncorrected_price)
    }
}

// --- Metrics Recording --- //

/// Extends the given labels with a base asset tag
pub(crate) fn extend_labels_with_base_asset(
    base_mint: &str,
    mut labels: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let base_token = Token::from_addr(base_mint);
    let base_asset = base_token.get_ticker().unwrap_or(base_mint.to_string());

    labels.insert(0, (BASE_ASSET_METRIC_TAG.to_string(), base_asset));
    labels
}

/// Extends the given labels with a side tag
pub(crate) fn extend_labels_with_side(
    side: &OrderSide,
    mut labels: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let side_label = if side == &OrderSide::Sell { "sell" } else { "buy" };
    labels.insert(0, (SIDE_TAG.to_string(), side_label.to_string()));
    labels
}

/// Record a volume metric with the given extra tags
pub(crate) fn record_volume_with_tags(
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
    order: &ExternalOrder,
    price: f64,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    // Record external order volume
    let base_mint = biguint_to_hex_addr(&order.base_mint);
    let quote_mint = biguint_to_hex_addr(&order.quote_mint);
    let labels = extend_labels_with_base_asset(&base_mint, labels.to_vec());

    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);

    // Calculate amount in quote using fixed point arithmetic
    let fixed_point_price = FixedPoint::from_f64_round_down(price);
    let quote_amount = order.get_quote_amount(fixed_point_price, relayer_fee);
    let base_amount = order.get_base_amount(fixed_point_price, relayer_fee);

    // Calculate the decimal quote volume to enforce the cap.
    let quote_token = Token::from_addr(&quote_mint);
    let quote_volume_decimal = quote_token.convert_to_decimal(quote_amount);
    let should_record_volume = quote_volume_decimal <= MAX_EXTERNAL_ORDER_QUOTE_VOLUME;

    if should_record_volume {
        // Record base/quote volumes using the original pattern.
        record_volume_with_tags(&base_mint, base_amount, EXTERNAL_ORDER_BASE_VOLUME, &labels);
        record_volume_with_tags(&quote_mint, quote_amount, EXTERNAL_ORDER_QUOTE_VOLUME, &labels);
    }

    // Always record request count metric.
    record_endpoint_metrics(&base_mint, NUM_EXTERNAL_MATCH_REQUESTS, &labels);

    Ok(())
}

/// Records metrics for the external match response (match bundle)
fn record_external_match_response_metrics(
    match_bundle: &AtomicMatchApiBundle,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let (base, quote) = match match_bundle.match_result.direction {
        OrderSide::Buy => (&match_bundle.receive, &match_bundle.send),
        OrderSide::Sell => (&match_bundle.send, &match_bundle.receive),
    };

    record_external_match_response_metrics_inner(
        &base.mint,
        base.amount,
        &quote.mint,
        quote.amount,
        labels,
    );
    Ok(())
}

/// Records the base/quote match volume metrics from scalar amounts.
///
/// Shared between the atomic and malleable flows: the atomic caller resolves
/// base/quote from the bundle's `send`/`receive` by direction, while the
/// malleable caller derives scalar amounts from the bounded match result.
fn record_external_match_response_metrics_inner(
    base_mint: &str,
    base_amount: u128,
    quote_mint: &str,
    quote_amount: u128,
    labels: &[(String, String)],
) {
    record_volume_with_tags(base_mint, base_amount, EXTERNAL_MATCH_BASE_VOLUME, labels);

    let labels = extend_labels_with_base_asset(base_mint, labels.to_vec());
    record_volume_with_tags(quote_mint, quote_amount, EXTERNAL_MATCH_QUOTE_VOLUME, &labels);
}

/// Compute a scalar quote amount from a fixed-point price and a scalar base
/// amount.
///
/// Mirrors `compute_quote_amount` in `connectors::rfqt::helpers` — kept local
/// to avoid a cross-module dependency from the telemetry layer into the
/// connector layer.
fn quote_amount_from_base(price: FixedPoint, base_amount: u128) -> u128 {
    let quote_amount_fp = price * Scalar::from(base_amount);
    scalar_to_u128(&quote_amount_fp.floor())
}

/// Records a counter metric with the given labels
pub(crate) fn record_endpoint_metrics(
    mint: &str,
    metric_name: &'static str,
    extra_labels: &[(String, String)],
) {
    let (asset, _) = get_asset_and_volume(mint, 0);
    let mut labels = vec![(ASSET_METRIC_TAG.to_string(), asset)];
    labels.extend(extra_labels.iter().cloned());
    metrics::counter!(metric_name, &labels).increment(1);
}

/// Records the fill ratio (matched quote amount / requested quote amount)
pub(crate) fn record_fill_ratio(
    requested_quote_amount: u128,
    matched_quote_amount: u128,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let fill_ratio = matched_quote_amount as f64 / requested_quote_amount as f64;
    metrics::gauge!(EXTERNAL_MATCH_FILL_RATIO, labels).set(fill_ratio);
    Ok(())
}

/// Records all metrics related to an external match request and response
pub(crate) fn record_external_match_metrics(
    order: &ExternalOrder,
    match_bundle: &AtomicMatchApiBundle,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    // Get decimal-corrected price
    let price = calculate_implied_price(match_bundle, true /* decimal_correct */)?;

    // Record request metrics
    if let Err(e) = record_external_match_request_metrics(order, price, labels) {
        warn!("Error recording request metrics: {e}");
    }

    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);

    // Record fill ratio metric
    let requested_quote_amount =
        order.get_quote_amount(FixedPoint::from_f64_round_down(price), relayer_fee);

    let matched_quote_amount = match_bundle.match_result.quote_amount;
    if let Err(e) = record_fill_ratio(requested_quote_amount, matched_quote_amount, labels) {
        warn!("Error recording fill ratio metric: {e}");
    }

    // Record response metrics
    if let Err(e) = record_external_match_response_metrics(match_bundle, labels) {
        warn!("Error recording response metrics: {e}");
    }

    Ok(())
}

/// Records all metrics related to an external match request and malleable
/// response.
///
/// Uses `max_base_amount` as the representative base amount for the match —
/// this matches the convention in `connectors::rfqt::helpers` (the RFQT layer
/// hands the solver a quote built from the upper bound of the range). Quote
/// amount is derived from `price_fp * max_base_amount`. The actual settled
/// amount is recorded separately from on-chain calldata by the chain-events
/// listener.
pub(crate) fn record_malleable_external_match_metrics(
    order: &ExternalOrder,
    match_bundle: &MalleableAtomicMatchApiBundle,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let price = match_bundle.match_result.price_fp.to_f64();
    let base_mint = &match_bundle.match_result.base_mint;
    let quote_mint = &match_bundle.match_result.quote_mint;

    // Pick the quoted upper bound as the representative match amount
    let base_amount = match_bundle.match_result.max_base_amount;
    let quote_amount = quote_amount_from_base(match_bundle.match_result.price_fp, base_amount);

    // Record request-side metrics (order volume + request count)
    if let Err(e) = record_external_match_request_metrics(order, price, labels) {
        warn!("Error recording malleable request metrics: {e}");
    }

    // Record fill ratio against the requested quote amount
    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);
    let requested_quote_amount =
        order.get_quote_amount(FixedPoint::from_f64_round_down(price), relayer_fee);
    if let Err(e) = record_fill_ratio(requested_quote_amount, quote_amount, labels) {
        warn!("Error recording malleable fill ratio metric: {e}");
    }

    // Record response-side match volume metrics
    record_external_match_response_metrics_inner(
        base_mint,
        base_amount,
        quote_mint,
        quote_amount,
        labels,
    );

    Ok(())
}

/// Record a counter metric for relayer requests that return a 500 status code
pub(crate) fn record_relayer_request_500(key_description: String, path: String) {
    let labels = vec![
        (KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description),
        (REQUEST_PATH_METRIC_TAG.to_string(), path),
    ];

    metrics::counter!(UNSUCCESSFUL_RELAYER_REQUEST_COUNT, &labels).increment(1);
}

/// Record a counter metric for quote requests for which the relayer could not
/// produce a quote
pub(crate) fn record_quote_not_found(key_description: String, base_mint: &str) {
    let base_token = Token::from_addr(base_mint);
    let base_asset = base_token.get_ticker().unwrap_or(base_mint.to_string());

    let labels = vec![
        (KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description),
        (BASE_ASSET_METRIC_TAG.to_string(), base_asset),
    ];

    metrics::counter!(QUOTE_NOT_FOUND_COUNT, &labels).increment(1);
}
