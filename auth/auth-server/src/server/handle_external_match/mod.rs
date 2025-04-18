//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use auth_server_api::{
    GasSponsorshipInfo, GasSponsorshipQueryParams, SponsoredMatchResponse, SponsoredQuoteResponse,
};
use bytes::Bytes;
use gas_sponsorship::refund_calculation::{
    apply_gas_sponsorship_to_exact_output_amount, remove_gas_sponsorship_from_quote,
    requires_exact_output_amount_update,
};
use http::{HeaderMap, Method, Response, StatusCode};
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalMatchRequest, ExternalMatchResponse, ExternalOrder,
    ExternalQuoteRequest, ExternalQuoteResponse,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_common::types::{token::Token, TimestampedPrice};
use renegade_constants::EXTERNAL_MATCH_RELAYER_FEE;
use tracing::{error, info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use super::helpers::{generate_quote_uuid, overwrite_response_body};
use super::Server;
use crate::error::AuthServerError;
use crate::store::helpers::generate_bundle_id;
use crate::store::BundleContext;
use crate::telemetry::helpers::{
    calculate_implied_price, extract_nullifier_from_match_bundle, record_relayer_request_500,
};
use crate::telemetry::labels::{GAS_SPONSORED_METRIC_TAG, SDK_VERSION_METRIC_TAG};
use crate::telemetry::{
    helpers::{
        await_settlement, record_endpoint_metrics, record_external_match_metrics, record_fill_ratio,
    },
    labels::{
        DECIMAL_CORRECTION_FIXED_METRIC_TAG, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT,
        KEY_DESCRIPTION_METRIC_TAG, REQUEST_ID_METRIC_TAG,
    },
};

mod gas_sponsorship;
pub use gas_sponsorship::contract_interaction::sponsorAtomicMatchSettleWithRefundOptionsCall;

// -------------
// | Constants |
// -------------

/// The header name for the SDK version
const SDK_VERSION_HEADER: &str = "x-renegade-sdk-version";

/// The default SDK version to use if the header is not set
const SDK_VERSION_DEFAULT: &str = "pre-v0.1.0";

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Handle an external quote request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let key_desc = self.authorize_request(path_str, &query_str, &headers, &body).await?;
        self.check_quote_rate_limit(key_desc.clone()).await?;

        // If necessary, ensure that the exact output amount requested in the order is
        // respected by any gas sponsorship applied to the relayer's quote
        let (external_quote_req, gas_sponsorship_info) = self
            .maybe_apply_gas_sponsorship_to_quote_request(key_desc.clone(), &body, &query_str)
            .await?;

        // Send the request to the relayer
        let req_body = serde_json::to_vec(&external_quote_req).map_err(AuthServerError::serde)?;
        let mut resp = self
            .send_admin_request(Method::POST, path_str, headers.clone(), req_body.clone().into())
            .await?;

        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_desc.clone(), path_str.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&resp, &key_desc, path_str, &req_body, &headers);
            return Ok(resp);
        }

        let sponsored_quote_response =
            self.maybe_apply_gas_sponsorship_to_quote_response(resp.body(), gas_sponsorship_info)?;

        overwrite_response_body(&mut resp, sponsored_quote_response.clone())?;

        let server_clone = self.clone();
        tokio::spawn(async move {
            // Cache the gas sponsorship info for the quote in Redis if it exists
            if let Err(e) =
                server_clone.cache_quote_gas_sponsorship_info(&sponsored_quote_response).await
            {
                error!("Error caching quote gas sponsorship info: {e}");
            }

            // Log the quote response & emit metrics
            if let Err(e) = server_clone.handle_quote_response(
                key_desc,
                &external_quote_req,
                &headers,
                &sponsored_quote_response,
            ) {
                warn!("Error handling quote: {e}");
            }
        });

        Ok(resp)
    }

    /// Handle an external quote-assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_quote_assembly_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let key_desc = self.authorize_request(path_str, &query_str, &headers, &body).await?;

        // Check the bundle rate limit
        let mut req: AssembleExternalMatchRequest =
            serde_json::from_slice(&body).map_err(AuthServerError::serde)?;
        self.check_bundle_rate_limit(key_desc.clone(), req.allow_shared).await?;

        // Update the request to remove the effects of gas sponsorship, if
        // necessary
        let gas_sponsorship_info =
            self.maybe_update_assembly_request_with_gas_sponsorship(&mut req).await?;
        let req_body = serde_json::to_vec(&req).map_err(AuthServerError::serde)?;

        // Send the request to the relayer
        let mut res = self
            .send_admin_request(Method::POST, path_str, headers.clone(), req_body.clone().into())
            .await?;

        let status = res.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_desc.clone(), path_str.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&res, &key_desc, path_str, &req_body, &headers);
            return Ok(res);
        }

        // Apply gas sponsorship to the resulting bundle, if necessary
        let sponsored_match_resp =
            self.maybe_apply_gas_sponsorship_to_match_response(res.body(), gas_sponsorship_info)?;
        overwrite_response_body(&mut res, sponsored_match_resp.clone())?;

        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_quote_assembly_bundle_response(
                    key_desc,
                    &req,
                    &headers,
                    &sponsored_match_resp,
                )
                .await
            {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(res)
    }

    /// Handle an external match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let key_description = self.authorize_request(path_str, &query_str, &headers, &body).await?;

        // Direct matches are always shared
        self.check_bundle_rate_limit(key_description.clone(), true /* shared */).await?;

        let (external_match_req, gas_sponsorship_info) = self
            .maybe_apply_gas_sponsorship_to_match_request(
                key_description.clone(),
                &body,
                &query_str,
            )
            .await?;

        let req_body = serde_json::to_vec(&external_match_req).map_err(AuthServerError::serde)?;

        // Send the request to the relayer, potentially sponsoring the gas costs

        let mut resp = self
            .send_admin_request(Method::POST, path_str, headers.clone(), req_body.clone().into())
            .await?;

        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_description.clone(), path_str.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(
                &resp,
                &key_description,
                path_str,
                &req_body,
                &headers,
            );
            return Ok(resp);
        }

        let sponsored_match_resp =
            self.maybe_apply_gas_sponsorship_to_match_response(resp.body(), gas_sponsorship_info)?;

        overwrite_response_body(&mut resp, sponsored_match_resp.clone())?;

        // Watch the bundle for settlement
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_direct_match_bundle_response(
                    key_description,
                    &external_match_req,
                    &headers,
                    &sponsored_match_resp,
                )
                .await
            {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(resp)
    }

    // --- Sponsorship --- //

    /// Potentially apply gas sponsorship to the given quote request,
    /// ensuring that any exact output amount requested in the order is
    /// respected.
    ///
    /// Returns the quote request, alongside the generated gas sponsorship info,
    /// if any.
    async fn maybe_apply_gas_sponsorship_to_quote_request(
        &self,
        key_desc: String,
        req_body: &[u8],
        query_str: &str,
    ) -> Result<(ExternalQuoteRequest, Option<GasSponsorshipInfo>), AuthServerError> {
        // Parse query params
        let query_params = serde_urlencoded::from_str::<GasSponsorshipQueryParams>(query_str)
            .map_err(AuthServerError::serde)?;

        // Parse request body
        let mut external_quote_req: ExternalQuoteRequest =
            serde_json::from_slice(req_body).map_err(AuthServerError::serde)?;

        let gas_sponsorship_info = self
            .maybe_apply_gas_sponsorship_to_order(
                key_desc,
                &mut external_quote_req.external_order,
                &query_params,
            )
            .await?;

        Ok((external_quote_req, gas_sponsorship_info))
    }

    /// Potentially apply gas sponsorship to the given
    /// external quote, returning the resulting `SponsoredQuoteResponse`
    fn maybe_apply_gas_sponsorship_to_quote_response(
        &self,
        resp_body: &[u8],
        gas_sponsorship_info: Option<GasSponsorshipInfo>,
    ) -> Result<SponsoredQuoteResponse, AuthServerError> {
        // Parse response body
        let external_quote_response: ExternalQuoteResponse =
            serde_json::from_slice(resp_body).map_err(AuthServerError::serde)?;

        if gas_sponsorship_info.is_none() {
            return Ok(SponsoredQuoteResponse {
                signed_quote: external_quote_response.signed_quote,
                gas_sponsorship_info: None,
            });
        }

        self.construct_sponsored_quote_response(
            external_quote_response,
            gas_sponsorship_info.unwrap(),
        )
    }

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
            .maybe_apply_gas_sponsorship_to_order(
                key_desc,
                &mut external_match_req.external_order,
                &query_params,
            )
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

    /// Generate gas sponsorship info for the given order if the query params
    /// call for it, and update the exact output amount requested in the order
    /// if necessary
    async fn maybe_apply_gas_sponsorship_to_order(
        &self,
        key_desc: String,
        order: &mut ExternalOrder,
        query_params: &GasSponsorshipQueryParams,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        let gas_sponsorship_info =
            self.maybe_generate_gas_sponsorship_info(key_desc, order, query_params).await?;

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

    /// Handle a bundle response from a quote assembly request
    async fn handle_quote_assembly_bundle_response(
        &self,
        key: String,
        req: &AssembleExternalMatchRequest,
        headers: &HeaderMap,
        resp: &SponsoredMatchResponse,
    ) -> Result<(), AuthServerError> {
        let original_order = &req.signed_quote.quote.order;
        let updated_order = req.updated_order.as_ref().unwrap_or(original_order);

        let request_id = uuid::Uuid::new_v4().to_string();
        let sdk_version = get_sdk_version(headers);
        if req.updated_order.is_some() {
            log_updated_order(&key, original_order, updated_order, &request_id, &sdk_version);
        }

        self.handle_bundle_response(
            key,
            updated_order,
            resp,
            Some(request_id),
            "assemble-external-match",
            req.allow_shared,
            sdk_version,
        )
        .await
    }

    /// Handle a bundle response from a direct match request
    async fn handle_direct_match_bundle_response(
        &self,
        key: String,
        req: &ExternalMatchRequest,
        headers: &HeaderMap,
        resp: &SponsoredMatchResponse,
    ) -> Result<(), AuthServerError> {
        let sdk_version = get_sdk_version(headers);
        self.handle_bundle_response(
            key,
            &req.external_order,
            resp,
            None,
            "request-external-match",
            true, // shared
            sdk_version,
        )
        .await
    }

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    #[allow(clippy::too_many_arguments)]
    async fn handle_bundle_response(
        &self,
        key: String,
        order: &ExternalOrder,
        resp: &SponsoredMatchResponse,
        request_id: Option<String>,
        endpoint: &str,
        shared_bundle: bool,
        sdk_version: String,
    ) -> Result<(), AuthServerError> {
        // Log the bundle
        let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        log_bundle(order, resp, &key, &request_id, endpoint, &sdk_version)?;

        // Note: if sponsored in-kind w/ refund going to the receiver,
        // the amounts in the match bundle will have been updated
        let SponsoredMatchResponse { match_bundle, is_sponsored, gas_sponsorship_info } = resp;

        let nullifier = extract_nullifier_from_match_bundle(match_bundle)?;
        let bundle_id = generate_bundle_id(&match_bundle.match_result, &nullifier)?;

        let bundle_ctx = BundleContext {
            key_description: key.clone(),
            request_id: bundle_id.clone(),
            sdk_version: sdk_version.clone(),
            gas_sponsorship_info: gas_sponsorship_info.clone(),
            is_sponsored: *is_sponsored,
            nullifier,
        };

        // Non-blocking write to bundle store
        let bundle_store = self.bundle_store.clone();
        tokio::spawn(async move {
            if let Err(e) = bundle_store.write(bundle_id, bundle_ctx).await {
                tracing::error!("bundle_store.write failed: {}", e);
            }
        });

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key.clone()),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.clone()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), is_sponsored.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), sdk_version.clone()),
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

        // If the bundle settles, increase the API user's a rate limit token balance
        let did_settle = await_settlement(match_bundle, &self.arbitrum_client).await?;
        if did_settle {
            self.add_bundle_rate_limit_token(key.clone(), shared_bundle).await;
            if let Some(gas_sponsorship_info) = gas_sponsorship_info {
                self.record_settled_match_sponsorship(
                    match_bundle,
                    gas_sponsorship_info,
                    key,
                    request_id,
                    sdk_version,
                )
                .await?;
            }
        }

        // Record metrics
        record_external_match_metrics(order, match_bundle, &labels, did_settle)?;

        Ok(())
    }

    /// Handle a quote response
    fn handle_quote_response(
        &self,
        key: String,
        req: &ExternalQuoteRequest,
        headers: &HeaderMap,
        resp: &SponsoredQuoteResponse,
    ) -> Result<(), AuthServerError> {
        let sdk_version = get_sdk_version(headers);

        // Log the quote parameters
        log_quote(resp, &key, &sdk_version)?;

        // Only proceed with metrics recording if sampled
        if !self.should_sample_metrics() {
            return Ok(());
        }

        // Get the decimal-corrected price
        let price: TimestampedPrice = resp.signed_quote.quote.price.clone().into();

        let relayer_fee = FixedPoint::from_f64_round_down(EXTERNAL_MATCH_RELAYER_FEE);

        // Calculate requested and matched quote amounts
        let requested_quote_amount = req
            .external_order
            .get_quote_amount(FixedPoint::from_f64_round_down(price.price), relayer_fee);

        let matched_quote_amount = resp.signed_quote.quote.match_result.quote_amount;

        // Record fill ratio metric
        let request_id = uuid::Uuid::new_v4();
        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.to_string()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), sdk_version),
        ];
        record_fill_ratio(requested_quote_amount, matched_quote_amount, &labels)?;

        // Record endpoint metrics
        let base_token = Token::from_addr_biguint(&req.external_order.base_mint);
        record_endpoint_metrics(&base_token.addr, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT, &labels);

        Ok(())
    }
}

// -------------------
// | Logging helpers |
// -------------------

/// Parse the SDK version from the given headers.
/// If unset or malformed, returns an empty string.
fn get_sdk_version(headers: &HeaderMap) -> String {
    headers
        .get(SDK_VERSION_HEADER)
        .map(|v| v.to_str().unwrap_or_default())
        .unwrap_or(SDK_VERSION_DEFAULT)
        .to_string()
}

/// Log a non-200 response from the relayer for the given request
fn log_unsuccessful_relayer_request(
    resp: &Response<Bytes>,
    key_description: &str,
    path: &str,
    req_body: &[u8],
    headers: &HeaderMap,
) {
    let status = resp.status();
    let text = String::from_utf8_lossy(resp.body()).to_string();
    let req_body = String::from_utf8_lossy(req_body).to_string();
    let sdk_version = get_sdk_version(headers);
    warn!(
        key_description = key_description,
        path = path,
        request_body = req_body,
        sdk_version = sdk_version,
        "Non-200 response from relayer: {status}: {text}",
    );
}

/// Log a quote
fn log_quote(
    resp: &SponsoredQuoteResponse,
    key_description: &str,
    sdk_version: &str,
) -> Result<(), AuthServerError> {
    let SponsoredQuoteResponse { signed_quote, gas_sponsorship_info } = resp;
    let match_result = signed_quote.match_result();
    let is_buy = match_result.direction;
    let recv = signed_quote.receive_amount();
    let send = signed_quote.send_amount();
    let is_sponsored = gas_sponsorship_info.is_some();
    let (refund_amount, refund_native_eth) = gas_sponsorship_info
        .as_ref()
        .map(|s| (s.gas_sponsorship_info.refund_amount, s.gas_sponsorship_info.refund_native_eth))
        .unwrap_or((0, false));

    info!(
            is_sponsored = is_sponsored,
            key_description = key_description,
            sdk_version = sdk_version,
            "Sending quote(is_buy: {is_buy}, receive: {} ({}), send: {} ({}), refund_amount: {} (refund_native_eth: {})) to client",
            recv.amount,
            recv.mint,
            send.amount,
            send.mint,
            refund_amount,
            refund_native_eth
        );

    Ok(())
}

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
