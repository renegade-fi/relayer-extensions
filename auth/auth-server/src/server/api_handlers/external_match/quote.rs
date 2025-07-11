//! Quote endpoint handler

use auth_server_api::{GasSponsorshipInfo, SponsoredQuoteResponse};
use bytes::Bytes;
use http::{Response, StatusCode};
use num_bigint::BigUint;
use renegade_api::http::external_match::{ExternalQuoteRequest, ExternalQuoteResponse};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_common::types::{price::TimestampedPrice, token::Token};
use renegade_constants::EXTERNAL_MATCH_RELAYER_FEE;
use renegade_util::hex::biguint_to_hex_addr;
use tracing::{error, info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::{
    error::AuthServerError,
    http_utils::request_response::overwrite_response_body,
    server::{
        api_handlers::{
            external_match::ExternalMatchRequestType, ticker_from_biguint, GLOBAL_MATCHING_POOL,
        },
        Server,
    },
    telemetry::{
        helpers::{record_endpoint_metrics, record_fill_ratio, record_quote_not_found},
        labels::{
            BASE_ASSET_METRIC_TAG, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT, KEY_DESCRIPTION_METRIC_TAG,
            REQUEST_ID_METRIC_TAG, SDK_VERSION_METRIC_TAG,
        },
        QUOTE_FILL_RATIO_IGNORE_THRESHOLD,
    },
};

use super::{RequestContext, ResponseContext};

// -----------------
// | Context Types |
// -----------------

/// The request context for a quote request
type QuoteRequestCtx = RequestContext<ExternalQuoteRequest>;

impl QuoteRequestCtx {
    /// Get the ticker for the quote request
    pub fn ticker(&self) -> Result<String, AuthServerError> {
        ticker_from_biguint(&self.body.external_order.base_mint)
    }
}

impl ExternalMatchRequestType for ExternalQuoteRequest {
    fn base_mint(&self) -> &BigUint {
        &self.external_order.base_mint
    }

    fn quote_mint(&self) -> &BigUint {
        &self.external_order.quote_mint
    }
}

/// The response context for a quote request
type QuoteResponseCtx = ResponseContext<ExternalQuoteRequest, ExternalQuoteResponse>;
/// The response context for a sponsored quote response
type SponsoredQuoteResponseCtx = ResponseContext<ExternalQuoteRequest, SponsoredQuoteResponse>;

impl SponsoredQuoteResponseCtx {
    /// Create a sponsored quote response context from a quote response context
    pub fn from_quote_response_ctx(
        sponsored_resp: SponsoredQuoteResponse,
        ctx: QuoteResponseCtx,
    ) -> Self {
        Self {
            path: ctx.path,
            query_str: ctx.query_str,
            user: ctx.user,
            sdk_version: ctx.sdk_version,
            headers: ctx.headers,
            request: ctx.request,
            status: ctx.status,
            response: Some(sponsored_resp),
            sponsorship_info: ctx.sponsorship_info,
            request_id: ctx.request_id,
        }
    }
}

// --------------------
// | Endpoint Handler |
// --------------------

impl Server {
    /// Handle an external quote request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // 1. Run the pre-request subroutines
        let mut ctx = self.preprocess_request(path, headers, body, query_str).await?;
        self.quote_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let resp = self.quote_post_request(raw_resp, ctx)?;
        Ok(resp)
    }

    // -------------------------------
    // | Request Pre/Post Processing |
    // -------------------------------

    /// Run endpoint handler subroutines before forwarding the request to the
    /// relayer
    #[instrument(skip_all)]
    async fn quote_pre_request(&self, ctx: &mut QuoteRequestCtx) -> Result<(), AuthServerError> {
        // Check the rate limit
        self.check_quote_rate_limit(ctx.user()).await?;
        self.route_quote_req(ctx).await?;

        // Apply gas sponsorship to the quote request
        let gas_sponsorship_info = self.sponsor_quote_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run endpoint handler subroutines after receiving the relayer's
    /// response
    ///
    /// Returns the auth server's response to the client
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    fn quote_post_request(
        &self,
        mut resp: Response<Bytes>,
        ctx: QuoteResponseCtx,
    ) -> Result<impl Reply, AuthServerError> {
        // If the relayer returns non-200, return the response directly
        let status = ctx.status();
        if status == StatusCode::NO_CONTENT {
            self.record_no_quote_found(ctx.clone());
        }
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Otherwise, apply gas sponsorship and post-process the quote
        let sponsored_resp = self.sponsor_response(&ctx)?;
        let should_stringify = ctx.should_stringify_body();
        overwrite_response_body(&mut resp, sponsored_resp.clone(), should_stringify)?;

        // Start a thread to record metrics and return
        let ctx = SponsoredQuoteResponseCtx::from_quote_response_ctx(sponsored_resp, ctx);
        self.record_quote_metrics(ctx);
        Ok(resp)
    }

    // -------------------
    // | Gas Sponsorship |
    // -------------------

    /// Route the quote request to the correct matching pool
    ///
    /// If execution costs limits have been exceeded by the bot server, we route
    /// to the global pool to take pressure off the quoters
    async fn route_quote_req(&self, ctx: &mut QuoteRequestCtx) -> Result<(), AuthServerError> {
        let ticker = ctx.ticker()?;
        let should_route_to_global = self.should_route_to_global(ctx.key_id(), &ticker).await?;
        if should_route_to_global {
            info!("Routing order to global matching pool");
            ctx.body_mut().matching_pool = Some(GLOBAL_MATCHING_POOL.to_string());
        }

        Ok(())
    }

    /// Apply gas sponsorship to the given quote request, if eligible. This
    /// ensures that any exact output amount requested in the order is
    /// respected.
    ///
    /// Returns the gas sponsorship info for the request, if any.
    #[instrument(skip_all)]
    async fn sponsor_quote_request(
        &self,
        ctx: &mut QuoteRequestCtx,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        // Apply gas sponsorship to the order
        let ctx_clone = ctx.clone();
        let req = ctx.body_mut();
        let gas_sponsorship_info =
            self.maybe_sponsor_order(&mut req.external_order, &ctx_clone).await?;

        Ok(gas_sponsorship_info)
    }

    /// Apply gas sponsorship to the given external quote response, returning
    /// the resulting `SponsoredQuoteResponse`
    #[instrument(skip_all)]
    fn sponsor_response(
        &self,
        ctx: &QuoteResponseCtx,
    ) -> Result<SponsoredQuoteResponse, AuthServerError> {
        let resp = ctx.response();
        if ctx.sponsorship_info().is_none() {
            return Ok(SponsoredQuoteResponse {
                signed_quote: resp.signed_quote,
                gas_sponsorship_info: None,
            });
        }

        let sponsorship_info = ctx.sponsorship_info().unwrap();
        self.construct_sponsored_quote_response(resp, sponsorship_info)
    }

    // -----------
    // | Metrics |
    // -----------

    /// Run the post-processing metrics subroutines for the quote endpoint
    fn record_quote_metrics(&self, ctx: SponsoredQuoteResponseCtx) {
        let server_clone = self.clone();
        tokio::spawn(async move {
            // Cache the gas sponsorship info for the quote in Redis if it exists
            let resp = ctx.response();
            if let Err(e) = server_clone.cache_quote_gas_sponsorship_info(&resp).await {
                error!("Error caching quote gas sponsorship info: {e}");
            }

            // Log the quote response & emit metrics
            if let Err(e) = server_clone.record_quote_metrics_helper(&ctx) {
                warn!("Error handling quote metrics: {e}");
            }
        });
    }

    /// Handle a quote response
    fn record_quote_metrics_helper(
        &self,
        ctx: &SponsoredQuoteResponseCtx,
    ) -> Result<(), AuthServerError> {
        log_quote(ctx)?;
        if !self.should_sample_metrics() {
            return Ok(());
        }

        // Get the decimal-corrected price
        let req = ctx.request();
        let resp = ctx.response();
        let ts_price: TimestampedPrice = resp.signed_quote.quote.price.clone().into();
        let price = ts_price.as_fixed_point();
        let relayer_fee = FixedPoint::from_f64_round_down(EXTERNAL_MATCH_RELAYER_FEE);

        // Calculate requested and matched quote amounts
        let requested_quote_amount = req.external_order.get_quote_amount(price, relayer_fee);
        let matched_quote_amount = resp.signed_quote.quote.match_result.quote_amount;

        // Record fill ratio metric
        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.user()),
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), ctx.sdk_version.clone()),
        ];
        record_fill_ratio(requested_quote_amount, matched_quote_amount, &labels)?;

        // Record endpoint metrics
        let base_token = Token::from_addr_biguint(&req.external_order.base_mint);
        record_endpoint_metrics(&base_token.addr, EXTERNAL_MATCH_QUOTE_REQUEST_COUNT, &labels);

        Ok(())
    }

    /// Handle a no quote found response
    fn record_no_quote_found(&self, ctx: QuoteResponseCtx) {
        let self_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = self_clone.record_no_quote_found_helper(&ctx).await {
                error!("Error recording no quote found metrics: {e}");
            }
        });
    }

    /// A helper for recording metrics when a quote is not found
    async fn record_no_quote_found_helper(
        &self,
        ctx: &QuoteResponseCtx,
    ) -> Result<(), AuthServerError> {
        let req = ctx.request();
        let order = &req.external_order;
        let base_mint = biguint_to_hex_addr(&order.base_mint);
        record_quote_not_found(ctx.user(), &base_mint);

        // Record a zero fill ratio
        let price_f64 = self.price_reporter_client.get_price(&base_mint, self.chain).await.unwrap();
        let price = FixedPoint::from_f64_round_down(price_f64);
        let relayer_fee = FixedPoint::zero();
        let quote_amt = order.get_quote_amount(price, relayer_fee);

        // We ignore excessively large quotes for telemetry, as they're likely spam
        if quote_amt >= QUOTE_FILL_RATIO_IGNORE_THRESHOLD {
            return Ok(());
        }

        // Record fill ratio metrics
        let labels = vec![
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.to_string()),
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.user()),
            (BASE_ASSET_METRIC_TAG.to_string(), base_mint),
        ];
        record_fill_ratio(quote_amt, 0 /* matched_quote_amount */, &labels)
            .expect("Failed to record fill ratio");

        Ok(())
    }
}

// -------------------
// | Logging helpers |
// -------------------

/// Log a quote
fn log_quote(ctx: &SponsoredQuoteResponseCtx) -> Result<(), AuthServerError> {
    let SponsoredQuoteResponse { signed_quote, gas_sponsorship_info } = ctx.response();
    let sdk_version = &ctx.sdk_version;
    let key_desc = &ctx.user();
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
            key_description = key_desc,
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
