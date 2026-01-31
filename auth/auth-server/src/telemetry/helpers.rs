//! Helper methods for capturing telemetry information throughout the auth
//! server

use alloy_primitives::Address;
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_constants::{
    DEFAULT_EXTERNAL_MATCH_RELAYER_FEE, NATIVE_ASSET_ADDRESS, NATIVE_ASSET_WRAPPER_TICKER,
};
use renegade_external_api::types::{
    ApiBoundedMatchResult, BoundedExternalMatchApiBundle, ExternalOrder,
};
use renegade_types_core::Token;
use renegade_util::hex::address_to_hex_string;
use tracing::warn;

use crate::{
    error::AuthServerError,
    server::{
        api_handlers::get_base_and_quote_amount_with_price, helpers::pick_base_and_quote_mints,
    },
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

/// Calculates the decimal-corrected quote per base price from a match result
/// Returns the price as an f64 decimal adjusted value, accounting for the
/// difference in decimal places between quote and base tokens
pub(crate) fn calculate_quote_per_base_price(
    match_result: &ApiBoundedMatchResult,
) -> Result<f64, AuthServerError> {
    let out_per_in_price = match_result.price_fp.to_f64();

    let input_mint = match_result.input_mint;
    let output_mint = match_result.output_mint;
    let (base_mint, quote_mint) = pick_base_and_quote_mints(input_mint, output_mint)?;

    let quote_per_base_price =
        if quote_mint == output_mint { out_per_in_price } else { 1.0 / out_per_in_price };

    let trades_native_asset =
        address_to_hex_string(&base_mint) == NATIVE_ASSET_ADDRESS.to_lowercase();
    let base_token = if trades_native_asset {
        Token::from_ticker(NATIVE_ASSET_WRAPPER_TICKER)
    } else {
        Token::from_alloy_address(&base_mint)
    };

    let quote_token = Token::from_alloy_address(&quote_mint);

    let base_decimals = base_token.get_decimals().ok_or_else(|| {
        AuthServerError::Serde(format!("No decimals for {}", base_token.get_addr()))
    })?;
    let quote_decimals = quote_token.get_decimals().ok_or_else(|| {
        AuthServerError::Serde(format!("No decimals for {}", quote_token.get_addr()))
    })?;

    let decimal_diff = quote_decimals as i32 - base_decimals as i32;
    Ok(quote_per_base_price * 10f64.powi(decimal_diff))
}

// --- Metrics Recording --- //

/// Extends the given labels with a base asset tag
pub(crate) fn extend_labels_with_base_asset(
    base_mint: &Address,
    mut labels: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let base_token = Token::from_alloy_address(base_mint);
    let base_asset = base_token.get_ticker().unwrap_or(address_to_hex_string(base_mint));

    labels.insert(0, (BASE_ASSET_METRIC_TAG.to_string(), base_asset));
    labels
}

/// Extends the given labels with a side tag derived from input/output mints
///
/// The side is from the external party's perspective:
/// - If base == output_mint → external receives base → external BUYS → "buy"
/// - If base == input_mint → external sends base → external SELLS → "sell"
pub(crate) fn extend_labels_with_side(
    input_mint: Address,
    output_mint: Address,
    mut labels: Vec<(String, String)>,
) -> Result<Vec<(String, String)>, AuthServerError> {
    let (base_mint, _) = pick_base_and_quote_mints(input_mint, output_mint)?;

    // External party buys if they receive the base token (base == output)
    let side_label = if base_mint == output_mint { "buy" } else { "sell" };
    labels.insert(0, (SIDE_TAG.to_string(), side_label.to_string()));

    Ok(labels)
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
    let (base_mint, quote_mint) = pick_base_and_quote_mints(order.input_mint, order.output_mint)?;
    let labels = extend_labels_with_base_asset(&base_mint, labels.to_vec());

    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);

    // Calculate base and quote amounts
    let (base_amount, quote_amount) =
        get_base_and_quote_amount_with_price(order, relayer_fee, price)?;

    // Calculate the decimal quote volume to enforce the cap.
    let quote_token = Token::from_alloy_address(&quote_mint);
    let quote_volume_decimal = quote_token.convert_to_decimal(quote_amount);
    let should_record_volume = quote_volume_decimal <= MAX_EXTERNAL_ORDER_QUOTE_VOLUME;

    let base_mint_str = address_to_hex_string(&base_mint);
    if should_record_volume {
        // Record base/quote volumes using the original pattern.
        record_volume_with_tags(&base_mint_str, base_amount, EXTERNAL_ORDER_BASE_VOLUME, &labels);
        record_volume_with_tags(
            &address_to_hex_string(&quote_mint),
            quote_amount,
            EXTERNAL_ORDER_QUOTE_VOLUME,
            &labels,
        );
    }

    // Always record request count metric.
    record_endpoint_metrics(&base_mint_str, NUM_EXTERNAL_MATCH_REQUESTS, &labels);

    Ok(())
}

/// Records metrics for the external match response (match bundle)
fn record_external_match_response_metrics(
    match_bundle: &BoundedExternalMatchApiBundle,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let (base_mint, quote_mint) = pick_base_and_quote_mints(
        match_bundle.match_result.input_mint,
        match_bundle.match_result.output_mint,
    )?;

    // TODO: Implement `get_default_base_amount` to get the base amount given by
    // the default input amount set in the match bundle calldata
    let base_amount = get_default_base_amount(match_bundle)?;
    record_volume_with_tags(
        &address_to_hex_string(&base_mint),
        base_amount,
        EXTERNAL_MATCH_BASE_VOLUME,
        labels,
    );

    let quote_amount = get_default_quote_amount(match_bundle)?;
    let labels = extend_labels_with_base_asset(&base_mint, labels.to_vec());
    record_volume_with_tags(
        &address_to_hex_string(&quote_mint),
        quote_amount,
        EXTERNAL_MATCH_QUOTE_VOLUME,
        &labels,
    );

    Ok(())
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
    match_bundle: &BoundedExternalMatchApiBundle,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    // Get decimal-corrected quote / base price
    let price = calculate_quote_per_base_price(&match_bundle.match_result)?;

    // Record request metrics
    if let Err(e) = record_external_match_request_metrics(order, price, labels) {
        warn!("Error recording request metrics: {e}");
    }

    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);

    // Record fill ratio metric
    let (_, requested_quote_amount) =
        get_base_and_quote_amount_with_price(order, relayer_fee, price)?;

    let matched_quote_amount = get_default_quote_amount(match_bundle)?;
    if let Err(e) = record_fill_ratio(requested_quote_amount, matched_quote_amount, labels) {
        warn!("Error recording fill ratio metric: {e}");
    }

    // Record response metrics
    if let Err(e) = record_external_match_response_metrics(match_bundle, labels) {
        warn!("Error recording response metrics: {e}");
    }

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

/// Get the base amount implied by the default setting of the
/// `externalPartyAmountIn` calldata field in the match bundle
pub(crate) fn get_default_base_amount(
    _match_bundle: &BoundedExternalMatchApiBundle,
) -> Result<u128, AuthServerError> {
    todo!()
}

/// Get the quote amount implied by the default setting of the
/// `externalPartyAmountIn` calldata field in the match bundle
pub(crate) fn get_default_quote_amount(
    _match_bundle: &BoundedExternalMatchApiBundle,
) -> Result<u128, AuthServerError> {
    todo!()
}
