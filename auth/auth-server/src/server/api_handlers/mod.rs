//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

mod external_match;
mod key_management;
mod order_book;
mod settlement;

use auth_server_api::{
    GasSponsorshipInfo, GasSponsorshipQueryParams, SponsoredMalleableMatchResponse,
    SponsoredMatchResponse,
};
use bytes::Bytes;
use external_match::{RequestContext, ResponseContext};
use http::{HeaderMap, Response};
use rand::Rng;
use renegade_api::http::external_match::{ExternalOrder, MalleableExternalMatchResponse};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_constants::EXTERNAL_MATCH_RELAYER_FEE;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::gas_sponsorship::refund_calculation::{
    apply_gas_sponsorship_to_exact_output_amount, requires_exact_output_amount_update,
};
use super::Server;
use crate::error::AuthServerError;
use crate::telemetry::helpers::calculate_implied_price;
use crate::telemetry::labels::{GAS_SPONSORED_METRIC_TAG, SDK_VERSION_METRIC_TAG};
use crate::telemetry::{
    helpers::record_external_match_metrics,
    labels::{KEY_DESCRIPTION_METRIC_TAG, REQUEST_ID_METRIC_TAG},
};

/// The header name for the SDK version
const SDK_VERSION_HEADER: &str = "x-renegade-sdk-version";
/// The default SDK version to use if the header is not set
const SDK_VERSION_DEFAULT: &str = "pre-v0.1.0";

/// A type alias for the response context for endpoints that return a match
/// bundle
///
/// This type is generic over request type, which allows us to use the same
/// handlers for endpoints with different request types but the same fundamental
/// match response type
type MatchBundleResponseCtx<Req> = ResponseContext<Req, SponsoredMatchResponse>;

/// Parse the SDK version from the given headers.
/// If unset or malformed, returns an empty string.
pub fn get_sdk_version(headers: &HeaderMap) -> String {
    headers
        .get(SDK_VERSION_HEADER)
        .map(|v| v.to_str().unwrap_or_default())
        .unwrap_or(SDK_VERSION_DEFAULT)
        .to_string()
}

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    // --- Sponsorship --- //

    /// Apply gas sponsorship to the given malleable match bundle, returning
    /// a `SponsoredMalleableMatchResponse  `
    fn maybe_apply_gas_sponsorship_to_malleable_match_bundle(
        &self,
        resp_body: &[u8],
        gas_sponsorship_info: Option<GasSponsorshipInfo>,
    ) -> Result<SponsoredMalleableMatchResponse, AuthServerError> {
        // Deserialize the response body from the relayer
        let match_resp: MalleableExternalMatchResponse =
            serde_json::from_slice(resp_body).map_err(AuthServerError::serde)?;
        if gas_sponsorship_info.is_none() {
            return Ok(SponsoredMalleableMatchResponse {
                match_bundle: match_resp.match_bundle,
                gas_sponsorship_info: None,
            });
        }

        // Construct the sponsored match response
        let info = gas_sponsorship_info.unwrap();
        let sponsored_match_resp =
            self.construct_sponsored_malleable_match_response(match_resp, info)?;
        Ok(sponsored_match_resp)
    }

    /// Generate gas sponsorship info for the given order if the query params
    /// call for it, and update the exact output amount requested in the order
    /// if necessary
    async fn maybe_sponsor_order<Req>(
        &self,
        order: &mut ExternalOrder,
        ctx: &RequestContext<Req>,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
    {
        // Parse query params
        let query = ctx.query();
        let query_params = serde_urlencoded::from_str::<GasSponsorshipQueryParams>(&query)
            .map_err(AuthServerError::serde)?;

        // Generate gas sponsorship info
        let user = ctx.user();
        let gas_sponsorship_info =
            self.generate_sponsorship_info(&user, order, &query_params).await?;

        // Subtract the refund amount from the exact output amount requested in the
        // order, so that the relayer produces a smaller quote which will
        // match the exact output amount after the refund is issued
        if let Some(ref gas_sponsorship_info) = gas_sponsorship_info
            && requires_exact_output_amount_update(order, gas_sponsorship_info)
        {
            info!(
                "Adjusting exact output amount requested in order to account for gas sponsorship"
            );
            apply_gas_sponsorship_to_exact_output_amount(order, gas_sponsorship_info);
        }

        Ok(gas_sponsorship_info)
    }

    // --- Bundle Tracking --- //

    /// Determines if the current request should be sampled for metrics
    /// collection
    pub fn should_sample_metrics(&self) -> bool {
        rand::thread_rng().gen_bool(self.metrics_sampling_rate)
    }

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    #[allow(clippy::too_many_arguments)]
    fn handle_bundle_response<Req>(
        &self,
        order: &ExternalOrder,
        ctx: &MatchBundleResponseCtx<Req>,
    ) -> Result<(), AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
    {
        // Log the bundle
        log_bundle(order, ctx)?;

        // Note: if sponsored in-kind w/ refund going to the receiver,
        // the amounts in the match bundle will have been updated
        let SponsoredMatchResponse { match_bundle, is_sponsored, .. } = ctx.response();

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.user()),
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.to_string()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), is_sponsored.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), ctx.sdk_version.clone()),
        ];

        // Record quote comparisons before settlement, if enabled
        if let Some(quote_metrics) = self.quote_metrics.clone() {
            let bundle_clone = match_bundle.clone();
            let labels_clone = labels.clone();

            // Record quote comparisons concurrently, so as to not interfere with awaiting
            // settlement
            tokio::spawn(async move {
                quote_metrics.record_quote_comparison(&bundle_clone, &labels_clone).await;
            });
        }

        // Record metrics
        record_external_match_metrics(order, &match_bundle, &labels)?;
        Ok(())
    }
}

// -------------------
// | Logging helpers |
// -------------------

/// Log a non-200 response from the relayer for the given request
pub fn log_unsuccessful_relayer_request(
    resp: &Response<Bytes>,
    key_description: &str,
    path: &str,
    headers: &HeaderMap,
) {
    let status = resp.status();
    let text = String::from_utf8_lossy(resp.body()).to_string();
    let sdk_version = get_sdk_version(headers);
    warn!(
        key_description = key_description,
        path = path,
        sdk_version = sdk_version,
        "Non-200 response from relayer: {status}: {text}",
    );
}

/// Log the bundle parameters
fn log_bundle<Req>(
    order: &ExternalOrder,
    ctx: &MatchBundleResponseCtx<Req>,
) -> Result<(), AuthServerError>
where
    Req: Serialize + for<'de> Deserialize<'de>,
{
    let SponsoredMatchResponse { match_bundle, is_sponsored, gas_sponsorship_info } =
        ctx.response();

    // Get the decimal-corrected price
    let price = calculate_implied_price(&match_bundle, true /* decimal_correct */)?;
    let price_fixed = FixedPoint::from_f64_round_down(price);

    let match_result = &match_bundle.match_result;
    let is_buy = match_result.direction;
    let recv = &match_bundle.receive;
    let send = &match_bundle.send;

    let relayer_fee = FixedPoint::from_f64_round_down(EXTERNAL_MATCH_RELAYER_FEE);

    // Get the base fill ratio
    let requested_base_amount = order.get_base_amount(price_fixed, relayer_fee);
    let response_base_amount = match_result.base_amount;
    let base_fill_ratio = response_base_amount as f64 / requested_base_amount as f64;

    // Get the quote fill ratio
    let requested_quote_amount = order.get_quote_amount(price_fixed, relayer_fee);
    let response_quote_amount = match_result.quote_amount;
    let quote_fill_ratio = response_quote_amount as f64 / requested_quote_amount as f64;

    // Get the gas sponsorship info
    let (refund_amount, refund_native_eth) = gas_sponsorship_info
        .as_ref()
        .map(|info| (info.refund_amount, info.refund_native_eth))
        .unwrap_or((0, false));

    let key_description = ctx.user();
    let request_id = ctx.request_id.to_string();
    info!(
            requested_base_amount = requested_base_amount,
            response_base_amount = response_base_amount,
            requested_quote_amount = requested_quote_amount,
            response_quote_amount = response_quote_amount,
            base_fill_ratio = base_fill_ratio,
            quote_fill_ratio = quote_fill_ratio,
            key_description = key_description,
            request_id = request_id,
            is_sponsored = is_sponsored,
            endpoint = ctx.path,
            sdk_version = ctx.sdk_version,
            "Sending bundle(is_buy: {}, recv: {} ({}), send: {} ({}), refund_amount: {} (refund_native_eth: {})) to client",
            is_buy,
            recv.amount,
            recv.mint,
            send.amount,
            send.mint,
            refund_amount,
            refund_native_eth
        );

    Ok(())
}
