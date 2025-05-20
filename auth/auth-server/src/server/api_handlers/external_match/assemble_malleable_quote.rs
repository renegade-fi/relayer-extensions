//! Assemble malleable quote endpoint handler

use auth_server_api::SponsoredMalleableMatchResponse;
use bytes::Bytes;
use http::Response;
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, MalleableExternalMatchResponse,
};
use tracing::instrument;
use warp::{reject::Rejection, reply::Reply};

use crate::{error::AuthServerError, http_utils::overwrite_response_body, server::Server};

use super::{RequestContext, ResponseContext};

// -----------------
// | Context Types |
// -----------------

/// The request context for an assemble malleable quote request
type AssembleMalleableQuoteRequestCtx = RequestContext<AssembleExternalMatchRequest>;
/// The response context for an assemble malleable quote request
type AssembleMalleableQuoteResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, MalleableExternalMatchResponse>;

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
    ) -> Result<impl Reply, Rejection> {
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
    async fn assemble_malleable_quote_pre_request(
        &self,
        ctx: &mut AssembleMalleableQuoteRequestCtx,
    ) -> Result<(), AuthServerError> {
        let allow_shared = ctx.body.allow_shared;
        let key_desc = ctx.user();
        self.check_bundle_rate_limit(key_desc, allow_shared).await?;

        // Apply gas sponsorship to the assembly request
        // The request type is the same between the standard and malleable assembly
        // endpoints so we can use the same modification method
        let gas_sponsorship_info = self.sponsor_assembly_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run the post-request subroutines for the assemble malleable quote
    /// endpoint
    fn assemble_malleable_quote_post_request(
        &self,
        mut resp: Response<Bytes>,
        ctx: &AssembleMalleableQuoteResponseCtx,
    ) -> Result<impl Reply, AuthServerError> {
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Apply gas sponsorship to the resulting bundle, if necessary
        let sponsored_match_resp = self.sponsor_malleable_assembly_response(ctx)?;
        overwrite_response_body(&mut resp, sponsored_match_resp.clone())?;

        // TODO: record bundle metrics
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
}
