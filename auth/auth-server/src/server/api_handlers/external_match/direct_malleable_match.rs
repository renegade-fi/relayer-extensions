//! A handler for requesting a malleable match directly--i.e. without first
//! requesting a quote

use auth_server_api::SponsoredMalleableMatchResponse;
use bytes::Bytes;
use renegade_api::http::external_match::{ExternalMatchRequest, MalleableExternalMatchResponse};
use renegade_util::get_current_time_millis;
use serde::Deserialize;
use tracing::{info, instrument, warn};
use warp::reject::Rejection;

use crate::{
    error::AuthServerError,
    http_utils::request_response::overwrite_response_body,
    server::{
        Server,
        api_handlers::{GLOBAL_MATCHING_POOL, external_match::BytesResponse},
    },
};

use super::{RequestContext, ResponseContext};

// -----------------
// | Context Types |
// -----------------

/// The request context for a direct malleable match request
type DirectMalleableMatchRequestCtx = RequestContext<ExternalMatchRequest>;

// Note: ExternalMatchRequestType is already implemented for
// ExternalMatchRequest in direct_match.rs, so we don't need to implement it
// again here

/// The response context for a direct malleable match request
type DirectMalleableMatchResponseCtx =
    ResponseContext<ExternalMatchRequest, MalleableExternalMatchResponse>;
/// The sponsored response context for a direct malleable match request
type SponsoredDirectMalleableMatchResponseCtx =
    ResponseContext<ExternalMatchRequest, SponsoredMalleableMatchResponse>;

impl SponsoredDirectMalleableMatchResponseCtx {
    /// Create a new sponsored direct malleable match response context from a
    /// direct malleable match response context and a sponsored malleable
    /// match response
    pub fn from_direct_malleable_match_response_ctx(
        sponsored_resp: SponsoredMalleableMatchResponse,
        ctx: DirectMalleableMatchResponseCtx,
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
            sponsorship_info_with_nonce: ctx.sponsorship_info_with_nonce,
            request_id: ctx.request_id,
        }
    }
}

// --- Query Params --- //

/// Typed query parameters for the direct malleable match endpoint
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct DirectMalleableMatchQueryParams {
    /// Whether to use the malleable match gas sponsor connector
    #[serde(default)]
    use_malleable_match_connector: Option<bool>,
}

impl ResponseContext<ExternalMatchRequest, MalleableExternalMatchResponse> {
    /// Get the `use_malleable_match_connector` flag from the query string
    pub fn use_malleable_match_connector(&self) -> bool {
        serde_urlencoded::from_str::<DirectMalleableMatchQueryParams>(&self.query_str)
            .unwrap_or_default()
            .use_malleable_match_connector
            .unwrap_or(false)
    }
}

// --------------------
// | Endpoint Handler |
// --------------------

impl Server {
    /// Handle an external direct malleable match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_direct_malleable_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<BytesResponse, Rejection> {
        // 1. Run the pre-request subroutines
        let mut ctx = self.preprocess_request(path, headers, body, query_str).await?;
        self.direct_malleable_match_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.direct_malleable_match_post_request(raw_resp, ctx)?;
        Ok(res)
    }

    // -------------------------------
    // | Request Pre/Post Processing |
    // -------------------------------

    /// Run the pre-request subroutines for the direct malleable match endpoint
    #[instrument(skip_all)]
    pub(crate) async fn direct_malleable_match_pre_request(
        &self,
        ctx: &mut DirectMalleableMatchRequestCtx,
    ) -> Result<(), AuthServerError> {
        // Check the rate limit
        if self.consume_bundle_rate_limit_token(&ctx.user()).await.is_err() {
            return Err(AuthServerError::no_match_found());
        };
        self.route_direct_malleable_match_req(ctx).await?;

        // Apply gas sponsorship to the match request
        let gas_sponsorship_info = self.sponsor_direct_malleable_match_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run the post-request subroutines for the direct malleable match endpoint
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    pub(crate) fn direct_malleable_match_post_request(
        &self,
        mut resp: BytesResponse,
        ctx: DirectMalleableMatchResponseCtx,
    ) -> Result<BytesResponse, AuthServerError> {
        // If the relayer returns non-200, return the response directly
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Apply gas sponsorship to the response
        let sponsored_resp = self.sponsor_direct_malleable_match_response(&ctx)?;
        let should_stringify = ctx.should_stringify_body();
        overwrite_response_body(&mut resp, sponsored_resp.clone(), should_stringify)?;

        // Record metrics
        let ctx =
            SponsoredDirectMalleableMatchResponseCtx::from_direct_malleable_match_response_ctx(
                sponsored_resp,
                ctx,
            );
        self.record_direct_malleable_match_metrics(ctx);
        Ok(resp)
    }

    // -------------------------
    // | Matching Pool Routing |
    // -------------------------

    /// Route the direct malleable match request to the correct matching pool
    ///
    /// If execution costs limits have been exceeded by the bot server, we route
    /// to the global pool to take pressure off the quoters
    async fn route_direct_malleable_match_req(
        &self,
        ctx: &mut DirectMalleableMatchRequestCtx,
    ) -> Result<(), AuthServerError> {
        // Use the trait method to avoid ambiguity with DirectMatchRequestCtx::ticker
        use crate::server::api_handlers::external_match::ExternalMatchRequestType;
        let ticker = ExternalMatchRequestType::base_ticker(&ctx.body)?;
        let should_route_to_global = self.should_route_to_global(ctx.key_id(), &ticker).await?;
        if should_route_to_global {
            info!("Routing order to global matching pool");
            ctx.body_mut().matching_pool = Some(GLOBAL_MATCHING_POOL.to_string());
        }

        Ok(())
    }

    // ---------------
    // | Sponsorship |
    // ---------------

    /// Apply gas sponsorship to the given direct malleable match request,
    /// returning the resulting `ExternalMatchRequest` and the generated gas
    /// sponsorship info.
    ///
    /// We use the gas sponsorship nonce to track bundle attribution, so we
    /// always return a `GasSponsorshipInfo` instance, even if the trade is
    /// not sponsored.
    #[instrument(skip_all)]
    async fn sponsor_direct_malleable_match_request(
        &self,
        ctx: &mut DirectMalleableMatchRequestCtx,
    ) -> Result<auth_server_api::GasSponsorshipInfo, AuthServerError> {
        let ctx_clone = ctx.clone();
        let req = ctx.body_mut();
        let gas_sponsorship_info =
            self.maybe_sponsor_order(&mut req.external_order, &ctx_clone).await?;

        Ok(gas_sponsorship_info)
    }

    /// Potentially apply gas sponsorship to the given direct malleable match
    /// response, returning the resulting `SponsoredMalleableMatchResponse`
    fn sponsor_direct_malleable_match_response(
        &self,
        ctx: &DirectMalleableMatchResponseCtx,
    ) -> Result<SponsoredMalleableMatchResponse, AuthServerError> {
        let resp = ctx.response();
        let sponsorship_info = ctx.sponsorship_info_with_nonce();
        if sponsorship_info.is_none() {
            return Ok(SponsoredMalleableMatchResponse {
                match_bundle: resp.match_bundle,
                gas_sponsorship_info: None,
            });
        }

        info!("Sponsoring malleable match bundle via gas sponsor");
        let (info, nonce) = sponsorship_info.unwrap();
        let use_malleable_match_connector = ctx.use_malleable_match_connector();
        let sponsored_match_resp = self.construct_sponsored_malleable_match_response(
            resp,
            info,
            nonce,
            use_malleable_match_connector,
        )?;

        Ok(sponsored_match_resp)
    }

    // -----------
    // | Metrics |
    // -----------

    /// Record metrics for the direct malleable match endpoint
    fn record_direct_malleable_match_metrics(&self, ctx: SponsoredDirectMalleableMatchResponseCtx) {
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.record_direct_malleable_match_metrics_helper(&ctx) {
                warn!("Error handling direct malleable match metrics: {e}");
            }
        });
    }

    /// A helper function to record metrics for the direct malleable match
    /// endpoint
    fn record_direct_malleable_match_metrics_helper(
        &self,
        ctx: &SponsoredDirectMalleableMatchResponseCtx,
    ) -> Result<(), AuthServerError> {
        // Because there is no price timestamp associated with a direct match,
        // we approximate it with the current time
        let price_timestamp = get_current_time_millis();
        // Record the bundle context in the store
        // Note: We use write_malleable_bundle_context which expects a price_timestamp
        // from signed_quote, but for direct matches we approximate it
        // We'll need to create a helper or adapt the existing method
        self.write_direct_malleable_bundle_context(price_timestamp, ctx)?;

        let req = ctx.request();
        let order = &req.external_order;
        self.handle_direct_malleable_bundle_response(order, ctx)
    }

    /// Write the bundle context for a direct malleable match to the store
    fn write_direct_malleable_bundle_context(
        &self,
        price_timestamp: u64,
        ctx: &SponsoredDirectMalleableMatchResponseCtx,
    ) -> Result<crate::bundle_store::BundleId, AuthServerError> {
        use crate::bundle_store::BundleContext;

        // We use the gas sponsorship nonce as the bundle ID. This is a per-bundle
        // unique identifier that we can use to attribute settlement
        let bundle_id = ctx
            .sponsorship_nonce()
            .ok_or_else(|| AuthServerError::gas_sponsorship("No sponsorship nonce found"))?;

        let gas_sponsorship_info = ctx.sponsorship_info_with_nonce();
        let is_sponsored = gas_sponsorship_info.is_some();

        let bundle_ctx = BundleContext {
            key_description: ctx.user(),
            bundle_id,
            request_id: ctx.request_id.to_string(),
            sdk_version: ctx.sdk_version.clone(),
            gas_sponsorship_info,
            is_sponsored,
            price_timestamp,
            assembled_timestamp: None,
        };

        self.bundle_store.write(bundle_ctx);
        Ok(bundle_id)
    }

    /// Handle a direct malleable bundle response
    fn handle_direct_malleable_bundle_response(
        &self,
        _order: &renegade_api::http::external_match::ExternalOrder,
        ctx: &SponsoredDirectMalleableMatchResponseCtx,
    ) -> Result<(), AuthServerError> {
        // Log the bundle
        info!(
            key_description = ctx.user(),
            request_id = ctx.request_id.to_string(),
            sdk_version = ctx.sdk_version,
            "Direct malleable match bundle forwarded to client"
        );

        // TODO: Record metrics
        // Note: record_external_match_metrics expects AtomicMatchApiBundle,
        // but we have MalleableAtomicMatchApiBundle. We need to create a
        // new metrics function for malleable bundles or adapt the existing one.
        Ok(())
    }
}
