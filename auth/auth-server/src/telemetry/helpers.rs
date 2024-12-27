//! Helper methods for capturing telemetry information throughout the auth
//! server

use std::time::Duration;

use alloy_sol_types::SolCall;
use contracts_common::types::MatchPayload;
use renegade_api::http::external_match::{
    AtomicMatchApiBundle, ExternalMatchResponse, ExternalOrder,
};
use renegade_arbitrum_client::{
    abi::{processAtomicMatchSettleCall, processAtomicMatchSettleWithReceiverCall},
    client::ArbitrumClient,
    helpers::deserialize_calldata,
};
use renegade_circuit_types::{fixed_point::FixedPoint, order::OrderSide, wallet::Nullifier};
use renegade_common::types::token::Token;
use renegade_constants::Scalar;
use renegade_util::hex::biguint_to_hex_addr;
use tracing::{info, warn};

use crate::{
    error::AuthServerError,
    telemetry::labels::{
        ASSET_METRIC_TAG, BASE_ASSET_METRIC_TAG, DECIMAL_CORRECTION_FIXED_METRIC_TAG,
        EXTERNAL_MATCH_BASE_VOLUME, EXTERNAL_MATCH_FILL_RATIO, EXTERNAL_MATCH_QUOTE_VOLUME,
        EXTERNAL_MATCH_SETTLED_BASE_VOLUME, EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
        EXTERNAL_ORDER_BASE_VOLUME, EXTERNAL_ORDER_QUOTE_VOLUME, KEY_DESCRIPTION_METRIC_TAG,
        NUM_EXTERNAL_MATCH_REQUESTS, REQUEST_ID_METRIC_TAG, SETTLEMENT_STATUS_TAG,
    },
};

/// The duration to await an atomic match settlement
pub const ATOMIC_SETTLEMENT_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Calculates the decimal-corrected quote per base price from a match bundle
/// Returns the price as an f64 decimal adjusted value, accounting for the
/// difference in decimal places between quote and base tokens
fn calculate_implied_price(match_bundle: &AtomicMatchApiBundle) -> Result<f64, AuthServerError> {
    let (base, quote) = match match_bundle.match_result.direction {
        OrderSide::Buy => (&match_bundle.receive, &match_bundle.send),
        OrderSide::Sell => (&match_bundle.send, &match_bundle.receive),
    };

    let base_token = Token::from_addr(&base.mint);
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
    let decimal_diff = quote_decimals as i32 - base_decimals as i32;
    let corrected_price = uncorrected_price * 10f64.powi(decimal_diff);

    Ok(corrected_price)
}

/// Extends the given labels with a base asset tag
fn extend_labels_with_base_asset(
    base_mint: &str,
    mut labels: Vec<(String, String)>,
) -> Vec<(String, String)> {
    let base_token = Token::from_addr(base_mint);
    let base_asset = base_token.get_ticker().unwrap_or(base_mint.to_string());

    labels.insert(0, (BASE_ASSET_METRIC_TAG.to_string(), base_asset));
    labels
}

/// Record a volume metric with the given extra tags
fn record_volume_with_tags(
    mint: &str,
    amount: u128,
    volume_metric_name: &'static str,
    extra_labels: &[(String, String)],
) {
    let (asset, volume) = get_asset_and_volume(mint, amount);
    let mut labels = vec![
        (ASSET_METRIC_TAG.to_string(), asset),
        (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
    ];
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

    // Calculate amount in quote using fixed point arithmetic
    let fixed_point_price = FixedPoint::from_f64_round_down(price);
    let quote_amount = order.get_quote_amount(fixed_point_price);
    let base_amount = order.get_base_amount(fixed_point_price);

    record_volume_with_tags(&base_mint, base_amount, EXTERNAL_ORDER_BASE_VOLUME, labels);

    let labels = extend_labels_with_base_asset(&base_mint, labels.to_vec());
    record_volume_with_tags(&quote_mint, quote_amount, EXTERNAL_ORDER_QUOTE_VOLUME, &labels);

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

/// Records metrics for the settlement of an external match
pub(crate) fn record_external_match_settlement_metrics(
    match_bundle: &AtomicMatchApiBundle,
    did_settle: bool,
    extra_labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let (base, quote) = match match_bundle.match_result.direction {
        OrderSide::Buy => (&match_bundle.receive, &match_bundle.send),
        OrderSide::Sell => (&match_bundle.send, &match_bundle.receive),
    };

    let mut labels = vec![(SETTLEMENT_STATUS_TAG.to_string(), did_settle.to_string())];
    labels.extend(extra_labels.iter().cloned());

    record_endpoint_metrics(
        &match_bundle.match_result.base_mint,
        NUM_EXTERNAL_MATCH_REQUESTS,
        &labels,
    );

    if did_settle {
        record_volume_with_tags(
            &base.mint,
            base.amount,
            EXTERNAL_MATCH_SETTLED_BASE_VOLUME,
            &labels,
        );

        let labels = extend_labels_with_base_asset(&base.mint, labels.to_vec());
        record_volume_with_tags(
            &quote.mint,
            quote.amount,
            EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
            &labels,
        );
    }

    Ok(())
}

/// Records a counter metric with the given labels
fn record_endpoint_metrics(
    mint: &str,
    metric_name: &'static str,
    extra_labels: &[(String, String)],
) {
    let (asset, _) = get_asset_and_volume(mint, 0);
    let mut labels = vec![
        (ASSET_METRIC_TAG.to_string(), asset),
        (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
    ];
    labels.extend(extra_labels.iter().cloned());
    metrics::counter!(metric_name, &labels).increment(1);
}

/// Records the fill ratio (match quote amount / order quote amount)
fn record_fill_ratio_metrics(
    match_bundle: &AtomicMatchApiBundle,
    requested_order_quote_amount: u128,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let (_, matched_quote) = match match_bundle.match_result.direction {
        OrderSide::Buy => (&match_bundle.receive, &match_bundle.send),
        OrderSide::Sell => (&match_bundle.send, &match_bundle.receive),
    };

    // Convert both amounts to decimal for accurate ratio calculation
    let quote_token = Token::from_addr(&matched_quote.mint);
    let matched_quote_amount = quote_token.convert_to_decimal(matched_quote.amount);
    let requested_quote_amount = quote_token.convert_to_decimal(requested_order_quote_amount);

    let fill_ratio = matched_quote_amount / requested_quote_amount;

    // Record the gauge metric
    metrics::gauge!(EXTERNAL_MATCH_FILL_RATIO, labels).set(fill_ratio);

    Ok(())
}

/// Records all metrics related to an external match request and response
pub(crate) async fn record_external_match_metrics(
    order: &ExternalOrder,
    match_resp: ExternalMatchResponse,
    key_description: String,
    did_settle: bool,
) -> Result<(), AuthServerError> {
    let request_id = uuid::Uuid::new_v4();
    let labels = vec![
        (KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description),
        (REQUEST_ID_METRIC_TAG.to_string(), request_id.to_string()),
    ];

    // Get decimal-corrected price
    let price = calculate_implied_price(&match_resp.match_bundle)?;

    // Record request metrics
    if let Err(e) = record_external_match_request_metrics(order, price, &labels) {
        warn!("Error recording request metrics: {e}");
    }

    let quote_amount = order.get_quote_amount(FixedPoint::from_f64_round_down(price));
    // Record fill ratio metric
    if let Err(e) = record_fill_ratio_metrics(&match_resp.match_bundle, quote_amount, &labels) {
        warn!("Error recording fill ratio metric: {e}");
    }

    // Record response metrics
    if let Err(e) = record_external_match_response_metrics(&match_resp.match_bundle, &labels) {
        warn!("Error recording response metrics: {e}");
    }

    if let Err(e) =
        record_external_match_settlement_metrics(&match_resp.match_bundle, did_settle, &labels)
    {
        warn!("Error recording settlement metrics: {e}");
    }

    Ok(())
}

/// Await the result of the atomic match settlement to be submitted on-chain
///
/// Returns `true` if the settlement succeeded on-chain, `false` otherwise
pub(crate) async fn await_settlement(
    match_bundle: &AtomicMatchApiBundle,
    arbitrum_client: &ArbitrumClient,
) -> Result<bool, AuthServerError> {
    let nullifier = extract_nullifier_from_match_bundle(match_bundle)?;
    let res = arbitrum_client.await_nullifier_spent(nullifier, ATOMIC_SETTLEMENT_TIMEOUT).await;

    let did_settle = res.is_ok();
    if !did_settle {
        info!("atomic match settlement not observed on-chain");
    }
    Ok(did_settle)
}

/// Extracts the nullifier from a match bundle's settlement transaction
///
/// This function attempts to decode the settlement transaction data in two
/// ways:
/// 1. As a standard atomic match settle call
/// 2. As a match settle with receiver call
fn extract_nullifier_from_match_bundle(
    match_bundle: &AtomicMatchApiBundle,
) -> Result<Nullifier, AuthServerError> {
    let tx_data = match_bundle
        .settlement_tx
        .data()
        .ok_or_else(|| AuthServerError::Serde("No data in settlement tx".to_string()))?;

    // Retrieve serialized match payload from the transaction data
    let serialized_match_payload =
        if let Ok(decoded) = processAtomicMatchSettleCall::abi_decode(tx_data, false) {
            decoded.internal_party_match_payload
        } else {
            let decoded = processAtomicMatchSettleWithReceiverCall::abi_decode(tx_data, false)
                .map_err(AuthServerError::serde)?;
            decoded.internal_party_match_payload
        };

    // Extract nullifier from the payload
    let match_payload = deserialize_calldata::<MatchPayload>(&serialized_match_payload)
        .map_err(AuthServerError::serde)?;
    let nullifier = Scalar::new(match_payload.valid_reblind_statement.original_shares_nullifier);

    Ok(nullifier)
}
