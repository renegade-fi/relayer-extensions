use crate::error::AuthServerError;
use crate::relayer_client::RelayerClient;
use crate::telemetry::labels::{
    EXTERNAL_MATCH_BUNDLE_VOLUME, EXTERNAL_ORDER_VOLUME, NUM_ATOMIC_MATCH_REQUESTS,
};
use renegade_api::http::external_match::{ExternalMatchRequest, ExternalMatchResponse};
use renegade_common::types::token::Token;
use renegade_common::types::TimestampedPrice;
use renegade_util::hex::biguint_to_hex_addr;
use tracing::warn;

use super::labels::{ASSET_METRIC_TAG, KEY_DESCRIPTION_METRIC_TAG};

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

/// Converts a decimal amount to token native units, accounting for the token's
/// decimals. This is the inverse operation of convert_to_decimal.
fn convert_from_decimal(token: &Token, decimal_amount: f64) -> u128 {
    let decimals = token.get_decimals().unwrap_or_default();
    (decimal_amount * (10u128.pow(decimals as u32) as f64)) as u128
}

/// Record a volume metric with the given extra tags
pub fn record_volume_with_tags(
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
pub async fn record_external_match_request_metrics(
    relayer_client: &RelayerClient,
    req: &ExternalMatchRequest,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    // Record external order volume
    let base_mint = biguint_to_hex_addr(&req.external_order.base_mint);
    let quote_mint = biguint_to_hex_addr(&req.external_order.quote_mint);

    if let Some(price) = relayer_client.get_binance_price(&base_mint, &quote_mint).await? {
        // Calculate amount in base
        let timestamped_price = TimestampedPrice::new(price);
        let fixed_point_price = timestamped_price.as_fixed_point();
        let order = req.external_order.to_order_with_price(fixed_point_price);

        // Calculate amount in quote
        let (_, volume) = get_asset_and_volume(&base_mint, order.amount);
        let quote_token = Token::from_addr(&quote_mint);
        let quote_amount = convert_from_decimal(&quote_token, volume * fixed_point_price.to_f64());

        record_volume_with_tags(&base_mint, order.amount, EXTERNAL_ORDER_VOLUME, labels);
        record_volume_with_tags(&quote_mint, quote_amount, EXTERNAL_ORDER_VOLUME, labels);
    }

    Ok(())
}

/// Records metrics for the external match response (match bundle)
pub fn record_external_match_response_metrics(
    resp: &ExternalMatchResponse,
    labels: &[(String, String)],
) -> Result<(), AuthServerError> {
    let recv = &resp.match_bundle.receive;
    let send = &resp.match_bundle.send;

    record_volume_with_tags(recv.mint.as_str(), recv.amount, EXTERNAL_MATCH_BUNDLE_VOLUME, labels);
    record_volume_with_tags(send.mint.as_str(), send.amount, EXTERNAL_MATCH_BUNDLE_VOLUME, labels);

    Ok(())
}

/// Records all metrics related to an external match request and response
pub async fn record_external_match_metrics(
    relayer_client: &RelayerClient,
    req_body: &[u8],
    resp_body: &[u8],
    key_description: String,
) -> Result<(), AuthServerError> {
    // Parse request and response
    let match_req =
        serde_json::from_slice::<ExternalMatchRequest>(req_body).map_err(AuthServerError::serde)?;
    let match_resp = serde_json::from_slice::<ExternalMatchResponse>(resp_body)
        .map_err(AuthServerError::serde)?;

    // Create labels with key description and asset
    let base_mint = biguint_to_hex_addr(&match_req.external_order.base_mint);
    let (asset, _) = get_asset_and_volume(&base_mint, 0);
    let labels = vec![
        (ASSET_METRIC_TAG.to_string(), asset),
        (KEY_DESCRIPTION_METRIC_TAG.to_string(), key_description),
    ];

    // Record atomic match request counter
    metrics::counter!(NUM_ATOMIC_MATCH_REQUESTS, &labels).increment(1);

    // Record request metrics
    if let Err(e) = record_external_match_request_metrics(relayer_client, &match_req, &labels).await
    {
        warn!("Error recording request metrics: {e}");
    }

    // Record response metrics
    if let Err(e) = record_external_match_response_metrics(&match_resp, &labels) {
        warn!("Error recording response metrics: {e}");
    }

    Ok(())
}
