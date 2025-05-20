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
use external_match::RequestContext;
use http::{HeaderMap, Response};
use rand::Rng;
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalMatchRequest, ExternalMatchResponse, ExternalOrder,
    MalleableExternalMatchResponse,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_constants::EXTERNAL_MATCH_RELAYER_FEE;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::gas_sponsorship::refund_calculation::{
    apply_gas_sponsorship_to_exact_output_amount, remove_gas_sponsorship_from_quote,
    requires_exact_output_amount_update,
};
use super::helpers::generate_quote_uuid;
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

/// Parse the SDK version from the given headers.
/// If unset or malformed, returns an empty string.
pub fn get_sdk_version(headers: &HeaderMap) -> String {
    headers
        .get(SDK_VERSION_HEADER)
        .map(|v| v.to_str().unwrap_or_default())
        .unwrap_or(SDK_VERSION_DEFAULT)
        .to_string()
}

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

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    // --- Sponsorship --- //

    /// Check if the given assembly request pertains to a sponsored quote,
    /// and if so, remove the effects of gas sponsorship from the signed quote,
    /// and ensure sponsorship is correctly applied to the updated order, if
    /// present.
    ///
    /// Returns the assembly request, and the gas sponsorship info, if any.
    async fn maybe_update_assembly_request_with_gas_sponsorship(
        &self,
        req: &mut AssembleExternalMatchRequest,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        let redis_key = generate_quote_uuid(&req.signed_quote);
        let gas_sponsorship_info = match self.read_gas_sponsorship_info_from_redis(redis_key).await
        {
            Err(e) => {
                error!("Error reading gas sponsorship info from Redis: {e}");
                None
            },
            Ok(gas_sponsorship_info) => gas_sponsorship_info,
        };

        if let Some(ref gas_sponsorship_info) = gas_sponsorship_info {
            // Reconstruct original signed quote
            if gas_sponsorship_info.requires_match_result_update() {
                let quote = &mut req.signed_quote.quote;
                remove_gas_sponsorship_from_quote(quote, gas_sponsorship_info);
            }

            // Ensure that the exact output amount is respected on the updated order
            if let Some(ref mut updated_order) = req.updated_order
                && requires_exact_output_amount_update(updated_order, gas_sponsorship_info)
            {
                apply_gas_sponsorship_to_exact_output_amount(updated_order, gas_sponsorship_info);
            }
        }

        Ok(gas_sponsorship_info)
    }

    /// Potentially apply gas sponsorship to the given match request, returning
    /// the resulting `ExternalMatchRequest` and the generated gas sponsorship
    /// info, if any.
    async fn maybe_apply_gas_sponsorship_to_match_request(
        &self,
        key_desc: String,
        req_body: &[u8],
        query_str: &str,
    ) -> Result<(ExternalMatchRequest, Option<GasSponsorshipInfo>), AuthServerError> {
        // Parse query params
        let query_params = serde_urlencoded::from_str::<GasSponsorshipQueryParams>(query_str)
            .map_err(AuthServerError::serde)?;

        // Parse request body
        let mut external_match_req: ExternalMatchRequest =
            serde_json::from_slice(req_body).map_err(AuthServerError::serde)?;

        let gas_sponsorship_info = self
            .maybe_sponsor_order(key_desc, &mut external_match_req.external_order, &query_params)
            .await?;

        Ok((external_match_req, gas_sponsorship_info))
    }

    /// Potentially apply gas sponsorship to the given
    /// external match response, returning the resulting
    /// `SponsoredMatchResponse`
    fn maybe_apply_gas_sponsorship_to_match_response(
        &self,
        resp_body: &[u8],
        gas_sponsorship_info: Option<GasSponsorshipInfo>,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        // Parse response body
        let external_match_resp: ExternalMatchResponse =
            serde_json::from_slice(resp_body).map_err(AuthServerError::serde)?;

        if gas_sponsorship_info.is_none() {
            return Ok(SponsoredMatchResponse {
                match_bundle: external_match_resp.match_bundle,
                is_sponsored: false,
                gas_sponsorship_info: None,
            });
        }

        info!("Sponsoring match bundle via gas sponsor");

        let sponsored_match_resp = self.construct_sponsored_match_response(
            external_match_resp,
            gas_sponsorship_info.unwrap(),
        )?;

        Ok(sponsored_match_resp)
    }

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

    /// Handle a bundle response from a quote assembly request
    fn handle_quote_assembly_bundle_response(
        &self,
        key: &str,
        req: &AssembleExternalMatchRequest,
        headers: &HeaderMap,
        resp: &SponsoredMatchResponse,
        request_id: &str,
    ) -> Result<(), AuthServerError> {
        let original_order = &req.signed_quote.quote.order;
        let updated_order = req.updated_order.as_ref().unwrap_or(original_order);

        let sdk_version = get_sdk_version(headers);
        if req.updated_order.is_some() {
            log_updated_order(key, original_order, updated_order, request_id, &sdk_version);
        }

        self.handle_bundle_response(
            key,
            updated_order,
            resp,
            request_id,
            "assemble-external-match",
            &sdk_version,
        )
    }

    /// Handle a bundle response from a direct match request
    fn handle_direct_match_bundle_response(
        &self,
        key: &str,
        req: &ExternalMatchRequest,
        headers: &HeaderMap,
        resp: &SponsoredMatchResponse,
        request_id: &str,
    ) -> Result<(), AuthServerError> {
        let sdk_version = get_sdk_version(headers);
        self.handle_bundle_response(
            key,
            &req.external_order,
            resp,
            request_id,
            "request-external-match",
            &sdk_version,
        )
    }

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    #[allow(clippy::too_many_arguments)]
    fn handle_bundle_response(
        &self,
        key: &str,
        order: &ExternalOrder,
        resp: &SponsoredMatchResponse,
        request_id: &str,
        endpoint: &str,
        sdk_version: &str,
    ) -> Result<(), AuthServerError> {
        // Log the bundle
        log_bundle(order, resp, key, request_id, endpoint, sdk_version)?;

        // Note: if sponsored in-kind w/ refund going to the receiver,
        // the amounts in the match bundle will have been updated
        let SponsoredMatchResponse { match_bundle, is_sponsored, .. } = resp;

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key.to_string()),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.to_string()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), is_sponsored.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), sdk_version.to_string()),
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
        record_external_match_metrics(order, match_bundle, &labels)?;

        Ok(())
    }
}

// -------------------
// | Logging helpers |
// -------------------

/// Log an updated order
fn log_updated_order(
    key: &str,
    original_order: &ExternalOrder,
    updated_order: &ExternalOrder,
    request_id: &str,
    sdk_version: &str,
) {
    let original_base_amount = original_order.base_amount;
    let updated_base_amount = updated_order.base_amount;
    let original_quote_amount = original_order.quote_amount;
    let updated_quote_amount = updated_order.quote_amount;
    info!(
            key_description = key,
            request_id = request_id,
            sdk_version = sdk_version,
            "Quote updated(original_base_amount: {}, updated_base_amount: {}, original_quote_amount: {}, updated_quote_amount: {})",
            original_base_amount, updated_base_amount, original_quote_amount, updated_quote_amount
        );
}

/// Log the bundle parameters
fn log_bundle(
    order: &ExternalOrder,
    resp: &SponsoredMatchResponse,
    key_description: &str,
    request_id: &str,
    endpoint: &str,
    sdk_version: &str,
) -> Result<(), AuthServerError> {
    let SponsoredMatchResponse { match_bundle, is_sponsored, gas_sponsorship_info } = resp;

    // Get the decimal-corrected price
    let price = calculate_implied_price(match_bundle, true /* decimal_correct */)?;
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
            endpoint = endpoint,
            sdk_version = sdk_version,
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
