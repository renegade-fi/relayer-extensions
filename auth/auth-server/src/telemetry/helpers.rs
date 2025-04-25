//! Helper methods for capturing telemetry information throughout the auth
//! server

use alloy_sol_types::SolCall;
use contracts_common::types::MatchPayload;
use renegade_api::http::external_match::{AtomicMatchApiBundle, ExternalOrder};
use renegade_arbitrum_client::{
    abi::{processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall},
    helpers::deserialize_calldata,
};
use renegade_circuit_types::{fixed_point::FixedPoint, order::OrderSide, wallet::Nullifier};
use renegade_common::types::token::Token;
use renegade_constants::{
    Scalar, EXTERNAL_MATCH_RELAYER_FEE, NATIVE_ASSET_ADDRESS, NATIVE_ASSET_WRAPPER_TICKER,
};
use renegade_util::hex::{biguint_from_hex_string, biguint_to_hex_addr};
use tracing::warn;

use crate::{
    error::AuthServerError,
    server::{
        handle_external_match::sponsorAtomicMatchSettleWithRefundOptionsCall, helpers::get_selector,
    },
    telemetry::labels::{
        ASSET_METRIC_TAG, BASE_ASSET_METRIC_TAG, EXTERNAL_MATCH_BASE_VOLUME,
        EXTERNAL_MATCH_FILL_RATIO, EXTERNAL_MATCH_QUOTE_VOLUME, EXTERNAL_ORDER_BASE_VOLUME,
        EXTERNAL_ORDER_QUOTE_VOLUME, OUR_NET_OUTPUT_TAG, OUR_OUTPUT_NET_OF_FEE_TAG,
        OUR_OUTPUT_NET_OF_GAS_TAG, OUR_PRICE_TAG, QUOTE_NET_OUTPUT_DIFF_BPS_METRIC,
        QUOTE_OUTPUT_NET_OF_FEE_DIFF_BPS_METRIC, QUOTE_OUTPUT_NET_OF_GAS_DIFF_BPS_METRIC,
        QUOTE_PRICE_DIFF_BPS_METRIC, SOURCE_NAME_TAG, SOURCE_NET_OUTPUT_TAG,
        SOURCE_OUTPUT_NET_OF_FEE_TAG, SOURCE_OUTPUT_NET_OF_GAS_TAG, SOURCE_PRICE_TAG,
        UNSUCCESSFUL_RELAYER_REQUEST_COUNT,
    },
};

use super::{
    labels::{
        KEY_DESCRIPTION_METRIC_TAG, NUM_EXTERNAL_MATCH_REQUESTS, REQUEST_PATH_METRIC_TAG, SIDE_TAG,
    },
    quote_comparison::QuoteComparison,
};

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

    let relayer_fee = FixedPoint::from_f64_round_down(EXTERNAL_MATCH_RELAYER_FEE);

    // Calculate amount in quote using fixed point arithmetic
    let fixed_point_price = FixedPoint::from_f64_round_down(price);
    let quote_amount = order.get_quote_amount(fixed_point_price, relayer_fee);
    let base_amount = order.get_base_amount(fixed_point_price, relayer_fee);

    record_volume_with_tags(&base_mint, base_amount, EXTERNAL_ORDER_BASE_VOLUME, labels);

    let labels = extend_labels_with_base_asset(&base_mint, labels.to_vec());
    record_volume_with_tags(&quote_mint, quote_amount, EXTERNAL_ORDER_QUOTE_VOLUME, &labels);

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

    record_volume_with_tags(&base.mint, base.amount, EXTERNAL_MATCH_BASE_VOLUME, labels);

    let labels = extend_labels_with_base_asset(&base.mint, labels.to_vec());
    record_volume_with_tags(&quote.mint, quote.amount, EXTERNAL_MATCH_QUOTE_VOLUME, &labels);

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
    match_bundle: &AtomicMatchApiBundle,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    // Get decimal-corrected price
    let price = calculate_implied_price(match_bundle, true /* decimal_correct */)?;

    // Record request metrics
    if let Err(e) = record_external_match_request_metrics(order, price, labels) {
        warn!("Error recording request metrics: {e}");
    }

    let relayer_fee = FixedPoint::from_f64_round_down(EXTERNAL_MATCH_RELAYER_FEE);

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

/// Record a counter metric for relayer requests that return a 500 status code
pub(crate) fn record_relayer_request_500(key_description: String, path: String) {
    let labels = vec![
        (KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description),
        (REQUEST_PATH_METRIC_TAG.to_string(), path),
    ];

    metrics::counter!(UNSUCCESSFUL_RELAYER_REQUEST_COUNT, &labels).increment(1);
}

// --- Settlement Processing --- //

/// Extracts the nullifier from a match bundle's settlement transaction
///
/// This function attempts to decode the settlement transaction data in two
/// ways:
/// 1. As a standard atomic match settle call
/// 2. As a match settle with receiver call
pub fn extract_nullifier_from_match_bundle(
    match_bundle: &AtomicMatchApiBundle,
) -> Result<Nullifier, AuthServerError> {
    let tx_data = match_bundle
        .settlement_tx
        .data()
        .ok_or(AuthServerError::serde("No data in settlement tx"))?;

    let selector = get_selector(tx_data)?;

    // Retrieve serialized match payload from the transaction data
    let serialized_match_payload = match selector {
        processAtomicMatchSettleCall::SELECTOR => {
            processAtomicMatchSettleCall::abi_decode(tx_data)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        processAtomicMatchSettleWithReceiverCall::SELECTOR => {
            processAtomicMatchSettleWithReceiverCall::abi_decode(tx_data)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        sponsorAtomicMatchSettleWithRefundOptionsCall::SELECTOR => {
            sponsorAtomicMatchSettleWithRefundOptionsCall::abi_decode(tx_data)
                .map_err(AuthServerError::serde)?
                .internal_party_match_payload
        },
        _ => {
            return Err(AuthServerError::serde("Invalid selector for settlement tx"));
        },
    };

    // Extract nullifier from the payload
    let match_payload = deserialize_calldata::<MatchPayload>(&serialized_match_payload)
        .map_err(AuthServerError::serde)?;
    let nullifier = Scalar::new(match_payload.valid_reblind_statement.original_shares_nullifier);

    Ok(nullifier)
}

// --- Quote Comparison --- //

/// Record a single quote comparison metric with all data as tags
pub(crate) fn record_quote_price_comparison(
    comparison: &QuoteComparison,
    side: OrderSide,
    extra_labels: &[(String, String)],
) {
    let side_label = if side == OrderSide::Sell { "sell" } else { "buy" };
    let base_token = Token::from_addr(&comparison.our_quote.base_mint);

    let mut labels = vec![
        (SIDE_TAG.to_string(), side_label.to_string()),
        (SOURCE_NAME_TAG.to_string(), comparison.source_quote.name.to_string()),
        (OUR_PRICE_TAG.to_string(), comparison.our_quote.price().to_string()),
        (SOURCE_PRICE_TAG.to_string(), comparison.source_quote.price().to_string()),
    ];
    labels.extend(extra_labels.iter().cloned());
    labels = extend_labels_with_base_asset(&base_token.get_addr(), labels);

    let price_diff_bps = comparison.price_diff_bps(side);
    metrics::gauge!(QUOTE_PRICE_DIFF_BPS_METRIC, labels.as_slice()).set(price_diff_bps);
}

/// Record a quote comparison net of gas cost
pub(crate) fn record_output_value_net_of_gas_comparison(
    comparison: &QuoteComparison,
    side: OrderSide,
    extra_labels: &[(String, String)],
) {
    let usdc_per_gas = comparison.usdc_per_gas;
    let output_diff_bps = comparison.output_value_net_of_gas_diff_bps(usdc_per_gas, side);

    let our_output_net_of_gas = comparison.our_quote.output_net_of_gas(usdc_per_gas, side);
    let source_output_net_of_gas = comparison.source_quote.output_net_of_gas(usdc_per_gas, side);

    let side_label = if side == OrderSide::Sell { "sell" } else { "buy" };
    let base_token = Token::from_addr(&comparison.our_quote.base_mint);
    let mut labels = vec![
        (SIDE_TAG.to_string(), side_label.to_string()),
        (SOURCE_NAME_TAG.to_string(), comparison.source_quote.name.to_string()),
        (OUR_OUTPUT_NET_OF_GAS_TAG.to_string(), our_output_net_of_gas.to_string()),
        (SOURCE_OUTPUT_NET_OF_GAS_TAG.to_string(), source_output_net_of_gas.to_string()),
    ];
    labels.extend(extra_labels.iter().cloned());
    labels = extend_labels_with_base_asset(&base_token.get_addr(), labels);

    metrics::gauge!(QUOTE_OUTPUT_NET_OF_GAS_DIFF_BPS_METRIC, labels.as_slice())
        .set(output_diff_bps);
}

/// Record a quote comparison net of fee
pub(crate) fn record_output_value_net_of_fee_comparison(
    comparison: &QuoteComparison,
    side: OrderSide,
    extra_labels: &[(String, String)],
) {
    let fee_diff_bps = comparison.output_value_net_of_fee_diff_bps(side);

    let our_output_net_of_fee = comparison.our_quote.output_net_of_fee(side);
    let source_output_net_of_fee = comparison.source_quote.output_net_of_fee(side);

    let side_label = if side == OrderSide::Sell { "sell" } else { "buy" };
    let base_token = Token::from_addr(&comparison.our_quote.base_mint);
    let mut labels = vec![
        (SIDE_TAG.to_string(), side_label.to_string()),
        (SOURCE_NAME_TAG.to_string(), comparison.source_quote.name.to_string()),
        (OUR_OUTPUT_NET_OF_FEE_TAG.to_string(), our_output_net_of_fee.to_string()),
        (SOURCE_OUTPUT_NET_OF_FEE_TAG.to_string(), source_output_net_of_fee.to_string()),
    ];
    labels.extend(extra_labels.iter().cloned());
    labels = extend_labels_with_base_asset(&base_token.get_addr(), labels);

    metrics::gauge!(QUOTE_OUTPUT_NET_OF_FEE_DIFF_BPS_METRIC, labels.as_slice()).set(fee_diff_bps);
}

/// Record a quote comparison net of gas and fee
pub(crate) fn record_net_output_value_comparison(
    comparison: &QuoteComparison,
    side: OrderSide,
    extra_labels: &[(String, String)],
) {
    let usdc_per_gas = comparison.usdc_per_gas;
    let net_output_diff_bps = comparison.net_output_value_diff_bps(usdc_per_gas, side);

    let our_net_output = comparison.our_quote.output_net_of_gas_and_fee(side, usdc_per_gas);
    let source_net_output = comparison.source_quote.output_net_of_gas_and_fee(side, usdc_per_gas);

    let side_label = if side == OrderSide::Sell { "sell" } else { "buy" };
    let base_token = Token::from_addr(&comparison.our_quote.base_mint);
    let mut labels = vec![
        (SIDE_TAG.to_string(), side_label.to_string()),
        (SOURCE_NAME_TAG.to_string(), comparison.source_quote.name.to_string()),
        (OUR_NET_OUTPUT_TAG.to_string(), our_net_output.to_string()),
        (SOURCE_NET_OUTPUT_TAG.to_string(), source_net_output.to_string()),
    ];
    labels.extend(extra_labels.iter().cloned());
    labels = extend_labels_with_base_asset(&base_token.get_addr(), labels);

    metrics::gauge!(QUOTE_NET_OUTPUT_DIFF_BPS_METRIC, labels.as_slice()).set(net_output_diff_bps);
}
