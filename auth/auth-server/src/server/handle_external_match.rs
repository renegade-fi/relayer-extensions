//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use bytes::Bytes;
use http::Method;
use tracing::{info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalMatchRequest, ExternalMatchResponse, ExternalOrder,
    ExternalQuoteResponse,
};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_common::types::{token::Token, TimestampedPrice};

use super::Server;
use crate::error::AuthServerError;
use crate::telemetry::{
    helpers::{
        await_settlement, record_endpoint_metrics, record_external_match_metrics, record_fill_ratio,
    },
    labels::{
        DECIMAL_CORRECTION_FIXED_METRIC_TAG, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT,
        KEY_DESCRIPTION_METRIC_TAG, REQUEST_ID_METRIC_TAG,
    },
};

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
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let key_desc = self.authorize_request(path.as_str(), &headers, &body).await?;
        self.check_rate_limit(key_desc.clone()).await?;

        // Send the request to the relayer
        let resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_quote_assembly_bundle_response(key_desc, &body, &resp_clone)
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
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let key_description = self.authorize_request(path.as_str(), &headers, &body).await?;
        self.check_rate_limit(key_description.clone()).await?;

        // Send the request to the relayer
        let resp =
            self.send_admin_request(Method::POST, path.as_str(), headers, body.clone()).await?;

        // Watch the bundle for settlement
        let resp_clone = resp.body().to_vec();
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone
                .handle_direct_match_bundle_response(key_description, &body, &resp_clone)
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
    ) -> Result<(), AuthServerError> {
        let req: AssembleExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;
        let order = req.signed_quote.quote.order;
        self.handle_bundle_response(key, order, resp).await
    }

    /// Handle a bundle response from a direct match request
    async fn handle_direct_match_bundle_response(
        &self,
        key: String,
        req: &[u8],
        resp: &[u8],
    ) -> Result<(), AuthServerError> {
        let req: ExternalMatchRequest =
            serde_json::from_slice(req).map_err(AuthServerError::serde)?;
        let order = req.external_order;
        self.handle_bundle_response(key, order, resp).await
    }

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    async fn handle_bundle_response(
        &self,
        key: String,
        order: ExternalOrder,
        resp: &[u8],
    ) -> Result<(), AuthServerError> {
        // Deserialize the response
        let match_resp: ExternalMatchResponse =
            serde_json::from_slice(resp).map_err(AuthServerError::serde)?;

        // Record quote comparison metrics before settlement
        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), key.clone()),
            (REQUEST_ID_METRIC_TAG.to_string(), uuid::Uuid::new_v4().to_string()),
            (DECIMAL_CORRECTION_FIXED_METRIC_TAG.to_string(), "true".to_string()),
        ];

        // Record quote comparisons
        self.quote_metrics
            .record_quote_comparison(&match_resp.match_bundle, labels.as_slice())
            .await;

        // If the bundle settles, increase the API user's a rate limit token balance
        let did_settle = await_settlement(&match_resp.match_bundle, &self.arbitrum_client).await?;
        if did_settle {
            self.add_rate_limit_token(key.clone()).await;
        }

        // Log the bundle and record metrics
        self.log_bundle(resp)?;
        record_external_match_metrics(&order, match_resp, key, did_settle).await?;

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

    /// Log the bundle parameters
    fn log_bundle(&self, bundle_bytes: &[u8]) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalMatchResponse>(bundle_bytes)
            .map_err(AuthServerError::serde)?;

        let match_result = resp.match_bundle.match_result;
        let is_buy = match_result.direction;
        let recv = resp.match_bundle.receive;
        let send = resp.match_bundle.send;
        info!(
            "Sending bundle(is_buy: {}, recv: {} ({}), send: {} ({})) to client",
            is_buy, recv.amount, recv.mint, send.amount, send.mint
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
