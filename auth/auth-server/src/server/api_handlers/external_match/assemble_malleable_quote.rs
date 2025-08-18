//! Assemble malleable quote endpoint handler

use auth_server_api::SponsoredMalleableMatchResponse;
use bytes::Bytes;
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, MalleableExternalMatchResponse,
};
use renegade_util::get_current_time_millis;
use tracing::{instrument, warn};
use warp::reject::Rejection;

use crate::{
    error::AuthServerError,
    http_utils::request_response::overwrite_response_body,
    server::{
        Server,
        api_handlers::external_match::{BytesResponse, assemble_quote::AssembleQuoteRequestCtx},
    },
};

use super::ResponseContext;

// -----------------
// | Context Types |
// -----------------

/// The response context for an assemble malleable quote request
type AssembleMalleableQuoteResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, MalleableExternalMatchResponse>;
/// The response context for an assemble malleable quote request with gas
/// sponsorship applied
pub(crate) type SponsoredAssembleMalleableQuoteResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, SponsoredMalleableMatchResponse>;

impl SponsoredAssembleMalleableQuoteResponseCtx {
    /// Create a new response context from an assemble malleable quote response
    /// context
    pub fn from_assemble_malleable_quote_response_ctx(
        sponsored_resp: SponsoredMalleableMatchResponse,
        ctx: AssembleMalleableQuoteResponseCtx,
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
    /// Handle an external malleable quote assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_assemble_malleable_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<BytesResponse, Rejection> {
        // 1. Run the pre-request subroutines
        let mut ctx = self.preprocess_request(path, headers, body, query_str).await?;
        self.assemble_malleable_quote_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.assemble_malleable_quote_post_request(raw_resp, &ctx)?;
        Ok(res)
    }

    // -------------------------------
    // | Request Pre/Post Processing |
    // -------------------------------

    /// Run the pre-request subroutines for the assemble malleable quote
    /// endpoint
    #[instrument(skip_all)]
    async fn assemble_malleable_quote_pre_request(
        &self,
        ctx: &mut AssembleQuoteRequestCtx,
    ) -> Result<(), AuthServerError> {
        let key_desc = &ctx.user();
        self.check_bundle_rate_limit(key_desc).await?;
        self.route_assembly_req(ctx).await?;

        // Apply gas sponsorship to the assembly request
        // The request type is the same between the standard and malleable assembly
        // endpoints so we can use the same modification method
        let gas_sponsorship_info = self.sponsor_assembly_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run the post-request subroutines for the assemble malleable quote
    /// endpoint
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    fn assemble_malleable_quote_post_request(
        &self,
        mut resp: BytesResponse,
        ctx: &AssembleMalleableQuoteResponseCtx,
    ) -> Result<BytesResponse, AuthServerError> {
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Apply gas sponsorship to the resulting bundle, if necessary
        let sponsored_match_resp = self.sponsor_malleable_assembly_response(ctx)?;
        let should_stringify = ctx.should_stringify_body();
        overwrite_response_body(&mut resp, sponsored_match_resp.clone(), should_stringify)?;

        // Record metrics
        let ctx =
            SponsoredAssembleMalleableQuoteResponseCtx::from_assemble_malleable_quote_response_ctx(
                sponsored_match_resp,
                ctx.clone(),
            );
        self.record_assemble_malleable_metrics(ctx);
        Ok(resp)
    }

    // ---------------
    // | Sponsorship |
    // ---------------

    /// Apply gas sponsorship to the given malleable match bundle, returning
    /// a `SponsoredMalleableMatchResponse  `
    fn sponsor_malleable_assembly_response(
        &self,
        ctx: &AssembleMalleableQuoteResponseCtx,
    ) -> Result<SponsoredMalleableMatchResponse, AuthServerError> {
        let resp = ctx.response();
        let sponsorship_info = ctx.sponsorship_info();
        if sponsorship_info.is_none() {
            return Ok(SponsoredMalleableMatchResponse {
                match_bundle: resp.match_bundle,
                gas_sponsorship_info: None,
            });
        }

        // Construct the sponsored match response
        let info = sponsorship_info.unwrap();
        let sponsored_match_resp = self.construct_sponsored_malleable_match_response(resp, info)?;
        Ok(sponsored_match_resp)
    }

    // -----------
    // | Metrics |
    // -----------

    /// Record metrics for the assemble malleable quote endpoint
    fn record_assemble_malleable_metrics(&self, ctx: SponsoredAssembleMalleableQuoteResponseCtx) {
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.record_assemble_malleable_metrics_helper(&ctx).await {
                warn!("Error handling assemble metrics: {e}");
            }
        });
    }

    /// A helper function to record metrics for the assemble malleable quote
    /// endpoint
    async fn record_assemble_malleable_metrics_helper(
        &self,
        ctx: &SponsoredAssembleMalleableQuoteResponseCtx,
    ) -> Result<(), AuthServerError> {
        // Record the bundle context in the store
        let assembled_timestamp = get_current_time_millis();
        self.write_malleable_bundle_context(Some(assembled_timestamp), ctx).await?;

        // TODO: Record metrics
        Ok(())
    }
}
