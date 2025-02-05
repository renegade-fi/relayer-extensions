//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use auth_server_api::{GasSponsorshipQueryParams, SponsoredMatchResponse};
use bytes::Bytes;
use http::{Method, StatusCode};
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, AtomicMatchApiBundle, ExternalMatchRequest,
    ExternalMatchResponse, ExternalOrder, ExternalQuoteResponse,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_common::types::{token::Token, TimestampedPrice};
use tracing::{info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use super::Server;
use crate::error::AuthServerError;
use crate::telemetry::helpers::calculate_implied_price;
use crate::telemetry::labels::GAS_SPONSORED_METRIC_TAG;
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
pub use gas_sponsorship::{sponsorAtomicMatchSettleCall, sponsorAtomicMatchSettleWithReceiverCall};

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
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let key_desc = self.authorize_request(path.as_str(), &headers, &body).await?;
        self.check_quote_rate_limit(key_desc.clone()).await?;

        // Send the request to the relayer
        let resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.handle_quote_response(key_desc, &body, &resp_clone) {
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
        query_params: GasSponsorshipQueryParams,
    ) -> Result<impl Reply, Rejection> {
        // Serialize the path + query params for auth
        let query_str = serde_urlencoded::to_string(&query_params).unwrap();
        let auth_path = if query_str.is_empty() {
            path.as_str().to_string()
        } else {
            format!("{}?{}", path.as_str(), query_str)
        };

        // Authorize the request
        let key_desc = self.authorize_request(&auth_path, &headers, &body).await?;
        self.check_bundle_rate_limit(key_desc.clone()).await?;

        let sponsorship_requested = query_params.use_gas_sponsorship.unwrap_or(false);
        let is_sponsored =
            sponsorship_requested && self.check_gas_sponsorship_rate_limit(key_desc.clone()).await;

        // Send the request to the relayer, potentially sponsoring the gas costs

        let mut resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let status = resp.status();
        if status != StatusCode::OK {
            warn!("Non-200 response from relayer: {}", status);
            return Ok(resp);
        }

        // We redirect the TX to the gas sponsor contract if the user explicitly
        // requested sponsorship, regardless of whether they are rate-limited.
        if sponsorship_requested {
            let refund_address =
                query_params.get_refund_address().map_err(AuthServerError::serde)?;

            self.mutate_response_for_gas_sponsorship(&mut resp, is_sponsored, refund_address)?;
        }

        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_quote_assembly_bundle_response(
                    key_desc,
                    &body,
                    &resp_clone,
                    sponsorship_requested,
                )
                .await
            {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(resp)
    }

    /// Handle an external match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_params: GasSponsorshipQueryParams,
    ) -> Result<impl Reply, Rejection> {
        // Serialize the path + query params for auth
        let query_str = serde_urlencoded::to_string(&query_params).unwrap();
        let auth_path = if query_str.is_empty() {
            path.as_str().to_string()
        } else {
            format!("{}?{}", path.as_str(), query_str)
        };

        // Authorize the request
        let key_description = self.authorize_request(&auth_path, &headers, &body).await?;
        self.check_bundle_rate_limit(key_description.clone()).await?;

        let sponsorship_requested = query_params.use_gas_sponsorship.unwrap_or(false);
        let is_sponsored = sponsorship_requested
            && self.check_gas_sponsorship_rate_limit(key_description.clone()).await;

        // Send the request to the relayer, potentially sponsoring the gas costs

        let mut resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let status = resp.status();
        if status != StatusCode::OK {
            warn!("Non-200 response from relayer: {}", status);
            return Ok(resp);
        }

        // We redirect the TX to the gas sponsor contract if the user explicitly
        // requested sponsorship, regardless of whether they are rate-limited.
        if sponsorship_requested {
            let refund_address =
                query_params.get_refund_address().map_err(AuthServerError::serde)?;

            self.mutate_response_for_gas_sponsorship(&mut resp, is_sponsored, refund_address)?;
        }

        // Watch the bundle for settlement
        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_direct_match_bundle_response(
                    key_description,
                    &body,
                    &resp_clone,
                    sponsorship_requested,
                )
                .await
            {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(resp)
    }

    // --- Bundle Tracking --- //

    /// Handle a bundle response from a quote assembly request
    async fn handle_quote_assembly_bundle_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
        sponsorship_requested: bool,
    ) -> Result<(), AuthServerError> {
        let req: AssembleExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;

        let original_order = &req.signed_quote.quote.order;
        let updated_order = req.updated_order.as_ref().unwrap_or(original_order);

        let request_id = uuid::Uuid::new_v4().to_string();
        if req.updated_order.is_some() {
            self.log_updated_order(&key, original_order, updated_order, &request_id);
        }

        self.handle_bundle_response(
            key,
            updated_order.clone(),
            resp,
            Some(request_id),
            sponsorship_requested,
        )
        .await
    }

    /// Handle a bundle response from a direct match request
    async fn handle_direct_match_bundle_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
        sponsorship_requested: bool,
    ) -> Result<(), AuthServerError> {
        let req: ExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;
        let order = req.external_order;
        self.handle_bundle_response(key, order, resp, None, sponsorship_requested).await
    }

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    async fn handle_bundle_response(
        &self,
        key: String,
        order: ExternalOrder,
        resp: &[u8],
        request_id: Option<String>,
        sponsorship_requested: bool,
    ) -> Result<(), AuthServerError> {
        // Log the bundle
        let request_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Deserialize the response
        let (is_sponsored, match_bundle) = if sponsorship_requested {
            let match_resp: SponsoredMatchResponse =
                serde_json::from_slice(resp).map_err(AuthServerError::serde)?;

            (match_resp.is_sponsored, match_resp.match_bundle)
        } else {
            let match_resp: ExternalMatchResponse =
                serde_json::from_slice(resp).map_err(AuthServerError::serde)?;

            (false, match_resp.match_bundle)
        };

        self.log_bundle(&order, &match_bundle, &key, &request_id)?;

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key.clone()),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.clone()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), is_sponsored.to_string()),
        ];

        // Record quote comparisons before settlement, if enabled
        if let Some(quote_metrics) = &self.quote_metrics {
            quote_metrics.record_quote_comparison(&match_bundle, labels.as_slice()).await;
        }

        // If the bundle settles, increase the API user's a rate limit token balance
        let did_settle = await_settlement(&match_bundle, &self.arbitrum_client).await?;
        if did_settle {
            self.add_bundle_rate_limit_token(key.clone()).await;
            self.record_settled_match_sponsorship(&match_bundle, is_sponsored, key, request_id)
                .await?;
        }

        // Record metrics
        record_external_match_metrics(&order, &match_bundle, &labels, did_settle).await?;

        Ok(())
    }

    // --- Logging --- //

    /// Log a quote
    fn log_quote(&self, quote_bytes: &[u8]) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalQuoteResponse>(quote_bytes)
            .map_err(AuthServerError::serde)?;

        let match_result = resp.signed_quote.match_result();
        let is_buy = match_result.direction;
        let recv = resp.signed_quote.receive_amount();
        let send = resp.signed_quote.send_amount();
        info!(
            "Sending quote(is_buy: {is_buy}, receive: {} ({}), send: {} ({})) to client",
            recv.amount, recv.mint, send.amount, send.mint
        );

        Ok(())
    }

    /// Log an updated order
    fn log_updated_order(
        &self,
        key: &str,
        original_order: &ExternalOrder,
        updated_order: &ExternalOrder,
        request_id: &str,
    ) {
        let original_base_amount = original_order.base_amount;
        let updated_base_amount = updated_order.base_amount;
        let original_quote_amount = original_order.quote_amount;
        let updated_quote_amount = updated_order.quote_amount;
        info!(
            key_description = key,
            request_id = request_id,
            "Quote updated(original_base_amount: {}, updated_base_amount: {}, original_quote_amount: {}, updated_quote_amount: {})",
            original_base_amount, updated_base_amount, original_quote_amount, updated_quote_amount
        );
    }

    /// Log the bundle parameters
    fn log_bundle(
        &self,
        order: &ExternalOrder,
        match_bundle: &AtomicMatchApiBundle,
        key_description: &str,
        request_id: &str,
    ) -> Result<(), AuthServerError> {
        // Get the decimal-corrected price
        let price = calculate_implied_price(match_bundle, true /* decimal_correct */)?;
        let price_fixed = FixedPoint::from_f64_round_down(price);

        let match_result = &match_bundle.match_result;
        let is_buy = match_result.direction;
        let recv = &match_bundle.receive;
        let send = &match_bundle.send;

        // Get the base fill ratio
        let requested_base_amount = order.get_base_amount(price_fixed);
        let response_base_amount = match_result.base_amount;
        let base_fill_ratio = response_base_amount as f64 / requested_base_amount as f64;

        // Get the quote fill ratio
        let requested_quote_amount = order.get_quote_amount(price_fixed);
        let response_quote_amount = match_result.quote_amount;
        let quote_fill_ratio = response_quote_amount as f64 / requested_quote_amount as f64;

        info!(
            requested_base_amount = requested_base_amount,
            response_base_amount = response_base_amount,
            requested_quote_amount = requested_quote_amount,
            response_quote_amount = response_quote_amount,
            base_fill_ratio = base_fill_ratio,
            quote_fill_ratio = quote_fill_ratio,
            key_description = key_description,
            request_id = request_id,
            "Sending bundle(is_buy: {}, recv: {} ({}), send: {} ({})) to client",
            is_buy,
            recv.amount,
            recv.mint,
            send.amount,
            send.mint
        );

        Ok(())
    }

    /// Handle a quote response
    fn handle_quote_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
    ) -> Result<(), AuthServerError> {
        // Log the quote parameters
        self.log_quote(resp)?;

        // Only proceed with metrics recording if sampled
        if !self.should_sample_metrics() {
            return Ok(());
        }

        // Parse request and response for metrics
        let req: ExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;
        let quote_resp: ExternalQuoteResponse =
            serde_json::from_slice(resp).map_err(AuthServerError::serde)?;

        // Get the decimal-corrected price
        let price: TimestampedPrice = quote_resp.signed_quote.quote.price.clone().into();

        // Calculate requested and matched quote amounts
        let requested_quote_amount =
            req.external_order.get_quote_amount(FixedPoint::from_f64_round_down(price.price));
        let matched_quote_amount = quote_resp.signed_quote.quote.match_result.quote_amount;

        // Record fill ratio metric
        let request_id = uuid::Uuid::new_v4();
        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key),
            (REQUEST_ID_METRIC_TAG.to_string(), request_id.to_string()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
        ];
        record_fill_ratio(requested_quote_amount, matched_quote_amount, &labels)?;

        // Record endpoint metrics
        let base_token = Token::from_addr_biguint(&req.external_order.base_mint);
        record_endpoint_metrics(&base_token.addr, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT, &labels);

        Ok(())
    }
}
