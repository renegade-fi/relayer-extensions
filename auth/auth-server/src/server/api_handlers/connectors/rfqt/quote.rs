//! RFQT Quote endpoint handler

use bytes::Bytes;
use http::HeaderMap;
use tracing::instrument;
use uuid::Uuid;
use warp::reject::Rejection;

use renegade_external_api::http::external_match::{
    ASSEMBLE_MATCH_BUNDLE_ROUTE, AssembleExternalMatchRequest, ExternalMatchResponse,
    ExternalQuoteRequest, ExternalQuoteResponse, GET_EXTERNAL_MATCH_QUOTE_ROUTE,
};

use auth_server_api::{SponsoredQuoteResponse, rfqt::RfqtQuoteRequest};

use crate::error::AuthServerError;
use crate::http_utils::request_response::overwrite_response_body;
use crate::http_utils::stringify_formatter::json_deserialize;
use crate::log_task;
use crate::logger::{Outcome, Task};
use crate::server::Server;
use crate::server::api_handlers::connectors::rfqt::RequestContextVariant;
use crate::server::api_handlers::connectors::rfqt::helpers::{
    create_direct_match_request, create_quote_request, should_use_malleable_calldata,
    transform_match_bundle_to_rfqt_response, transform_quote_to_assemble_malleable_ctx,
};
use crate::server::api_handlers::external_match::{BytesResponse, RequestContext, ResponseContext};
use crate::server::api_handlers::get_sdk_version;

impl Server {
    /// Handle the RFQT Quote endpoint (`POST /rfqt/v3/quote`).
    pub async fn handle_rfqt_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<BytesResponse, Rejection> {
        let (ctx, rfqt_request) = self.rfqt_pre_request(path, headers, body, query_str).await?;

        match ctx {
            RequestContextVariant::Malleable(quote_ctx) => {
                log_task!(
                    Task::RfqtQuote,
                    Outcome::Started,
                    subject = "request",
                    chain = %self.chain,
                    mode = "malleable",
                    "POST /rfqt/v3/quote"
                );
                let res = self.handle_quote_and_assemble(quote_ctx, rfqt_request).await?;
                log_task!(
                    Task::RfqtQuote,
                    Outcome::Ok,
                    subject = "request",
                    chain = %self.chain,
                    mode = "malleable",
                    "POST /rfqt/v3/quote completed"
                );
                Ok(res)
            },
            RequestContextVariant::Direct(assemble_ctx) => {
                log_task!(
                    Task::RfqtQuote,
                    Outcome::Started,
                    subject = "request",
                    chain = %self.chain,
                    mode = "direct",
                    "POST /rfqt/v3/quote"
                );
                let res = self.handle_direct_match(assemble_ctx, rfqt_request).await?;
                log_task!(
                    Task::RfqtQuote,
                    Outcome::Ok,
                    subject = "request",
                    chain = %self.chain,
                    mode = "direct",
                    "POST /rfqt/v3/quote completed"
                );
                Ok(res)
            },
        }
    }

    /// Build request context for an RFQT request, before the request is
    /// proxied to the relayer.
    ///
    /// 1. Authorize using the original RFQT request body (for HMAC validation).
    /// 2. Construct the `RequestContext` with the transformed request body —
    ///    either an external quote (malleable path) or a direct-order assemble.
    /// 3. Validate that one side of the pair is USDC and apply the per-key
    ///    relayer fee.
    #[instrument(skip_all)]
    async fn rfqt_pre_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<(RequestContextVariant, RfqtQuoteRequest), AuthServerError> {
        let path = path.as_str().to_string();
        let (key_desc, key_id) = self.authorize_request(&path, &query_str, &headers, &body).await?;
        let sdk_version = get_sdk_version(&headers);

        let rfq_request: RfqtQuoteRequest = json_deserialize(&body, false /* stringify */)?;

        let ctx = if should_use_malleable_calldata(&query_str) {
            let external_quote_request = create_quote_request(&rfq_request)?;
            self.validate_request_body(&external_quote_request)?;

            let mut ctx = RequestContext {
                path: GET_EXTERNAL_MATCH_QUOTE_ROUTE.to_string(),
                query_str: query_str.clone(),
                sdk_version: sdk_version.clone(),
                headers: headers.clone(),
                user: key_desc.clone(),
                key_id,
                body: external_quote_request,
                sponsorship_info: None,
                request_id: Uuid::new_v4(),
            };
            self.set_relayer_fee(&mut ctx).await?;
            RequestContextVariant::Malleable(ctx)
        } else {
            let assemble_request = create_direct_match_request(&rfq_request)?;
            self.validate_request_body(&assemble_request)?;

            let mut ctx = RequestContext {
                path: ASSEMBLE_MATCH_BUNDLE_ROUTE.to_string(),
                query_str: query_str.clone(),
                sdk_version: sdk_version.clone(),
                headers: headers.clone(),
                user: key_desc.clone(),
                key_id,
                body: assemble_request,
                sponsorship_info: None,
                request_id: Uuid::new_v4(),
            };
            self.set_relayer_fee(&mut ctx).await?;
            RequestContextVariant::Direct(ctx)
        };

        Ok((ctx, rfq_request))
    }

    /// Handle a quote-and-assemble (malleable) RFQT request.
    ///
    /// The flow mirrors v1: run the quote step, synchronously sponsor the
    /// returned quote, write the sponsorship cache so the subsequent assemble
    /// step can reattach the gas-sponsor nonce, and then run the v2 assembly
    /// pipeline (which itself reads the cache, fills the bundle, and emits
    /// telemetry via `record_external_match_metrics`).
    async fn handle_quote_and_assemble(
        &self,
        mut quote_req_ctx: RequestContext<ExternalQuoteRequest>,
        rfqt_request: RfqtQuoteRequest,
    ) -> Result<BytesResponse, Rejection> {
        // 1. Quote pre-request (rate limits, routing, gas-sponsor application to the
        //    order).
        self.quote_pre_request(&mut quote_req_ctx).await?;

        // 2. Proxy the quote request to the relayer.
        let (quote_raw_resp, quote_resp_ctx) =
            self.forward_request::<_, ExternalQuoteResponse>(quote_req_ctx.clone()).await?;

        // 3. If the relayer rejected the quote, forward the response and stop.
        if !quote_resp_ctx.is_success() {
            return Ok(quote_raw_resp);
        }

        // 4. Build the sponsored quote synchronously and persist the cached sponsorship
        //    info to Redis before assembling. The regular quote endpoint caches
        //    asynchronously via a spawned task, which would race the internal assemble
        //    step here.
        let (sponsored_quote, cached_info) = self.sponsor_rfqt_quote_response(&quote_resp_ctx)?;
        if let Some(cached_info) = cached_info {
            self.cache_quote_gas_sponsorship_info(&sponsored_quote, cached_info).await?;
        }

        // 5. Transform into the assemble request context (QuotedOrder).
        let mut assemble_req_ctx =
            transform_quote_to_assemble_malleable_ctx(sponsored_quote, quote_req_ctx)?;

        // 6. Run the v2 assembly pipeline (rate limits, gas sponsorship, forwarding,
        //    metric emission).
        self.assembly_pre_request(&mut assemble_req_ctx).await?;
        let (assemble_raw_resp, assemble_resp_ctx) =
            self.forward_request::<_, ExternalMatchResponse>(assemble_req_ctx).await?;
        let assemble_res =
            self.assembly_post_request(assemble_raw_resp, assemble_resp_ctx.clone())?;

        // 7. Overwrite the response body with the RFQT-shaped response.
        let assemble_res =
            self.rfqt_post_request_malleable(&rfqt_request, assemble_res, &assemble_resp_ctx)?;

        Ok(assemble_res)
    }

    /// Handle a direct (non-malleable) RFQT request via a single assemble call.
    async fn handle_direct_match(
        &self,
        mut ctx: RequestContext<AssembleExternalMatchRequest>,
        req: RfqtQuoteRequest,
    ) -> Result<BytesResponse, Rejection> {
        self.assembly_pre_request(&mut ctx).await?;
        let (raw_resp, resp_ctx) = self.forward_request::<_, ExternalMatchResponse>(ctx).await?;
        let res = self.assembly_post_request(raw_resp, resp_ctx.clone())?;
        let res = self.rfqt_post_request_direct(&req, res, &resp_ctx)?;
        Ok(res)
    }

    /// Overwrite the relayer response body with the malleable RFQT response
    /// shape (price + min/max receive/send populated).
    pub(crate) fn rfqt_post_request_malleable(
        &self,
        req: &RfqtQuoteRequest,
        mut resp: BytesResponse,
        ctx: &ResponseContext<AssembleExternalMatchRequest, ExternalMatchResponse>,
    ) -> Result<BytesResponse, AuthServerError> {
        if !ctx.is_success() {
            return Ok(resp);
        }

        let match_bundle = ctx.response().match_bundle;
        let rfqt_response =
            transform_match_bundle_to_rfqt_response(&match_bundle, req, true /* malleable */)?;
        overwrite_response_body(&mut resp, rfqt_response, true /* stringify */)?;
        Ok(resp)
    }

    /// Overwrite the relayer response body with the direct (single-amount)
    /// RFQT response shape (price + min/max fields stripped).
    pub(crate) fn rfqt_post_request_direct(
        &self,
        req: &RfqtQuoteRequest,
        mut resp: BytesResponse,
        ctx: &ResponseContext<AssembleExternalMatchRequest, ExternalMatchResponse>,
    ) -> Result<BytesResponse, AuthServerError> {
        if !ctx.is_success() {
            return Ok(resp);
        }

        let match_bundle = ctx.response().match_bundle;
        let rfqt_response = transform_match_bundle_to_rfqt_response(
            &match_bundle,
            req,
            false, // malleable
        )?;
        overwrite_response_body(&mut resp, rfqt_response, true /* stringify */)?;
        Ok(resp)
    }

    /// Build the sponsored quote shape needed for the internal RFQT assemble
    /// flow, without modifying the shared external quote handler.
    fn sponsor_rfqt_quote_response(
        &self,
        ctx: &ResponseContext<ExternalQuoteRequest, ExternalQuoteResponse>,
    ) -> Result<
        (SponsoredQuoteResponse, Option<crate::server::gas_sponsorship::CachedSponsorshipInfo>),
        AuthServerError,
    > {
        let resp = ctx.response();
        if let Some(sponsorship_info) = ctx.sponsorship_info() {
            let (sponsored, cached) =
                self.construct_sponsored_quote_response(resp, sponsorship_info)?;
            return Ok((sponsored, cached));
        }

        let sponsored =
            SponsoredQuoteResponse { signed_quote: resp.signed_quote, gas_sponsorship_info: None };
        Ok((sponsored, None))
    }
}
