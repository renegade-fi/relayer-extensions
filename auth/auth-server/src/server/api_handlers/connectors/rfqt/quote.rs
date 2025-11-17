//! RFQT Quote endpoint handler

use bytes::Bytes;
use http::HeaderMap;
use tracing::instrument;
use uuid::Uuid;
use warp::reject::Rejection;

use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalMatchRequest, ExternalMatchResponse,
    ExternalQuoteRequest, MalleableExternalMatchResponse, REQUEST_EXTERNAL_MATCH_ROUTE,
    REQUEST_EXTERNAL_QUOTE_ROUTE,
};

use auth_server_api::rfqt::RfqtQuoteRequest;

use crate::error::AuthServerError;
use crate::http_utils::request_response::overwrite_response_body;
use crate::http_utils::stringify_formatter::json_deserialize;
use crate::server::Server;
use crate::server::api_handlers::connectors::rfqt::helpers::{
    create_direct_match_request, create_quote_request, should_use_malleable_calldata,
    transform_match_bundle_to_rfqt_response, transform_quote_to_assemble_malleable_ctx,
};
use crate::server::api_handlers::connectors::rfqt::{MatchBundle, RequestContextVariant};
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
                self.handle_quote_and_assemble(quote_ctx, rfqt_request.clone()).await
            },
            RequestContextVariant::Direct(match_ctx) => {
                self.handle_direct_match(match_ctx, rfqt_request.clone()).await
            },
        }
    }

    /// Build request context for an RFQT request, before the request is
    /// proxied to the relayer
    ///
    /// This method handles the specific case where we need to:
    /// 1. Authorize using the original RFQT request body (for HMAC validation)
    /// 2. Construct the RequestContext with the transformed request body
    ///    (either quote or direct match based on query params)
    #[instrument(skip_all)]
    async fn rfqt_pre_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<(RequestContextVariant, RfqtQuoteRequest), AuthServerError> {
        // Authorize using the original RFQT body (for proper HMAC validation)
        let path = path.as_str().to_string();
        let (key_desc, key_id) = self.authorize_request(&path, &query_str, &headers, &body).await?;
        let sdk_version = get_sdk_version(&headers);

        // Parse the original RFQT request body
        let rfq_request: RfqtQuoteRequest = json_deserialize(&body, false /* stringify */)?;

        // Determine transformation based on query params
        let ctx = if should_use_malleable_calldata(&query_str) {
            // Transform to external quote request
            let external_quote_request = create_quote_request(rfq_request.clone())?;
            self.validate_request_body(&external_quote_request)?;

            let mut ctx = RequestContext {
                path: REQUEST_EXTERNAL_QUOTE_ROUTE.to_string(),
                query_str: query_str.clone(),
                sdk_version: sdk_version.clone(),
                headers: headers.clone(),
                user: key_desc.clone(),
                key_id,
                body: external_quote_request,
                sponsorship_info: None,
                request_id: Uuid::new_v4(),
            };

            // Set the relayer fee
            self.set_relayer_fee(&mut ctx).await?;
            RequestContextVariant::Malleable(ctx)
        } else {
            // Transform to external match request
            let external_match_request = create_direct_match_request(rfq_request.clone())?;
            self.validate_request_body(&external_match_request)?;

            let mut ctx = RequestContext {
                path: REQUEST_EXTERNAL_MATCH_ROUTE.to_string(),
                query_str: query_str.clone(),
                sdk_version: sdk_version.clone(),
                headers: headers.clone(),
                user: key_desc.clone(),
                key_id,
                body: external_match_request,
                sponsorship_info: None,
                request_id: Uuid::new_v4(),
            };

            // Set the relayer fee
            self.set_relayer_fee(&mut ctx).await?;
            RequestContextVariant::Direct(ctx)
        };

        Ok((ctx, rfq_request))
    }

    /// Handle a quote and assemble malleable match request
    async fn handle_quote_and_assemble(
        &self,
        mut quote_req_ctx: RequestContext<ExternalQuoteRequest>,
        rfqt_request: RfqtQuoteRequest,
    ) -> Result<BytesResponse, Rejection> {
        // 1. Run the pre-request subroutines
        self.quote_pre_request(&mut quote_req_ctx).await?;

        // 2. Proxy the request to the relayer
        let (quote_raw_resp, quote_resp_ctx) = self.forward_request(quote_req_ctx.clone()).await?;

        // 3. Run the quote post-request subroutines
        let _ = self.quote_post_request(quote_raw_resp, quote_resp_ctx.clone())?;

        // 4. Build assemble malleable quote request
        let mut assemble_req_ctx =
            transform_quote_to_assemble_malleable_ctx(quote_resp_ctx.response(), quote_req_ctx)?;

        // 5. Run assemble pre-request subroutines
        self.assemble_malleable_quote_pre_request(&mut assemble_req_ctx).await?;

        // 6. Proxy the request to the relayer
        let (assemble_raw_resp, assemble_req_ctx) = self.forward_request(assemble_req_ctx).await?;

        // 7. Run assemble post-request subroutines
        let assemble_res =
            self.assemble_malleable_quote_post_request(assemble_raw_resp, &assemble_req_ctx)?;

        // 8. RFQT-specific post-processing
        let assemble_res =
            self.rfqt_post_request_malleable(&rfqt_request, assemble_res, &assemble_req_ctx)?;

        Ok(assemble_res)
    }

    /// Handle a direct match request
    async fn handle_direct_match(
        &self,
        mut ctx: RequestContext<ExternalMatchRequest>,
        req: RfqtQuoteRequest,
    ) -> Result<BytesResponse, Rejection> {
        // 1. Run the pre-request subroutines
        self.direct_match_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.direct_match_post_request(raw_resp, ctx.clone())?;

        // 4. RFQT-specific post-processing
        let res = self.rfqt_post_request_direct(&req, res, &ctx)?;

        Ok(res)
    }

    /// Transform a malleable match response to an RFQT response
    pub(crate) fn rfqt_post_request_malleable(
        &self,
        req: &RfqtQuoteRequest,
        mut resp: BytesResponse,
        ctx: &ResponseContext<AssembleExternalMatchRequest, MalleableExternalMatchResponse>,
    ) -> Result<BytesResponse, AuthServerError> {
        if !ctx.is_success() {
            return Ok(resp);
        }

        let external_match_body = resp.body();
        let external_match_resp: MalleableExternalMatchResponse =
            serde_json::from_slice(external_match_body).map_err(AuthServerError::serde)?;

        let rfqt_response = transform_match_bundle_to_rfqt_response(
            MatchBundle::Malleable(external_match_resp.match_bundle),
            req,
        )?;

        overwrite_response_body(&mut resp, rfqt_response, true /* stringify */)?;
        Ok(resp)
    }

    /// Transform a direct match response to an RFQT response
    pub(crate) fn rfqt_post_request_direct(
        &self,
        req: &RfqtQuoteRequest,
        mut resp: BytesResponse,
        ctx: &ResponseContext<ExternalMatchRequest, ExternalMatchResponse>,
    ) -> Result<BytesResponse, AuthServerError> {
        if !ctx.is_success() {
            return Ok(resp);
        }

        let external_match_body = resp.body();
        let external_match_resp: ExternalMatchResponse =
            serde_json::from_slice(external_match_body).map_err(AuthServerError::serde)?;

        let rfqt_response = transform_match_bundle_to_rfqt_response(
            MatchBundle::Direct(external_match_resp.match_bundle),
            req,
        )?;

        overwrite_response_body(&mut resp, rfqt_response, true /* stringify */)?;
        Ok(resp)
    }
}
