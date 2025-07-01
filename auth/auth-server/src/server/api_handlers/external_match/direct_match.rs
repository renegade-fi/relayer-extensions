//! Direct match endpoint handler

use auth_server_api::{GasSponsorshipInfo, SponsoredMatchResponse};
use bytes::Bytes;
use http::Response;
use num_bigint::BigUint;
use renegade_api::http::external_match::{ExternalMatchRequest, ExternalMatchResponse};
use renegade_util::get_current_time_millis;
use tracing::{info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::{
    error::AuthServerError,
    http_utils::overwrite_response_body,
    server::{
        api_handlers::{
            external_match::ExternalMatchRequestType, ticker_from_biguint, GLOBAL_MATCHING_POOL,
        },
        Server,
    },
};

use super::{RequestContext, ResponseContext};

// -----------------
// | Context Types |
// -----------------

/// The request context for a direct match request
type DirectMatchRequestCtx = RequestContext<ExternalMatchRequest>;
impl DirectMatchRequestCtx {
    /// Get the ticker from the request
    pub fn ticker(&self) -> Result<String, AuthServerError> {
        ticker_from_biguint(&self.body.external_order.base_mint)
    }
}

impl ExternalMatchRequestType for ExternalMatchRequest {
    fn base_mint(&self) -> &BigUint {
        &self.external_order.base_mint
    }

    fn quote_mint(&self) -> &BigUint {
        &self.external_order.quote_mint
    }
}

/// The response context for a direct match request
type DirectMatchResponseCtx = ResponseContext<ExternalMatchRequest, ExternalMatchResponse>;
/// The sponsored response context for a direct match request
type SponsoredDirectMatchResponseCtx =
    ResponseContext<ExternalMatchRequest, SponsoredMatchResponse>;

impl SponsoredDirectMatchResponseCtx {
    /// Create a new sponsored direct match response context from a direct
    /// match response context and a sponsored match response
    pub fn from_direct_match_response_ctx(
        sponsored_resp: SponsoredMatchResponse,
        ctx: DirectMatchResponseCtx,
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
    /// Handle an external match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // 1. Run the pre-request subroutines
        let mut ctx = self.preprocess_request(path, headers, body, query_str).await?;
        self.direct_match_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.direct_match_post_request(raw_resp, ctx)?;
        Ok(res)
    }

    // -------------------------------
    // | Request Pre/Post Processing |
    // -------------------------------

    /// Run the pre-request subroutines for the direct match endpoint
    #[instrument(skip_all)]
    async fn direct_match_pre_request(
        &self,
        ctx: &mut DirectMatchRequestCtx,
    ) -> Result<(), AuthServerError> {
        // Check the rate limit
        // Direct matches are always shared
        self.check_bundle_rate_limit(ctx.user(), true /* shared */).await?;
        self.route_direct_match_req(ctx).await?;

        // Apply gas sponsorship to the match request
        let gas_sponsorship_info = self.sponsor_direct_match_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run the post-request subroutines for the direct match endpoint
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    fn direct_match_post_request(
        &self,
        mut resp: Response<Bytes>,
        ctx: DirectMatchResponseCtx,
    ) -> Result<impl Reply, AuthServerError> {
        // If the relayer returns non-200, return the response directly
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Apply gas sponsorship to the response
        let sponsored_resp = self.sponsor_direct_match_response(&ctx)?;
        overwrite_response_body(&mut resp, sponsored_resp.clone())?;

        // Record metrics
        let ctx =
            SponsoredDirectMatchResponseCtx::from_direct_match_response_ctx(sponsored_resp, ctx);
        self.record_direct_match_metrics(ctx);
        Ok(resp)
    }

    // -------------------------
    // | Matching Pool Routing |
    // -------------------------

    /// Route the direct match request to the correct matching pool
    ///
    /// If execution costs limits have been exceeded by the bot server, we route
    /// to the global pool to take pressure off the quoters
    async fn route_direct_match_req(
        &self,
        ctx: &mut DirectMatchRequestCtx,
    ) -> Result<(), AuthServerError> {
        let ticker = ctx.ticker()?;
        let limit_exceeded = self.check_execution_cost_exceeded(&ticker).await;
        if limit_exceeded {
            info!("Routing order to global matching pool");
            ctx.body_mut().matching_pool = Some(GLOBAL_MATCHING_POOL.to_string());
        }

        Ok(())
    }

    // ---------------
    // | Sponsorship |
    // ---------------

    /// Apply gas sponsorship to the given match request, returning the
    /// resulting `ExternalMatchRequest` and the generated gas sponsorship
    /// info, if any
    #[instrument(skip_all)]
    async fn sponsor_direct_match_request(
        &self,
        ctx: &mut DirectMatchRequestCtx,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        let ctx_clone = ctx.clone();
        let req = ctx.body_mut();
        let gas_sponsorship_info =
            self.maybe_sponsor_order(&mut req.external_order, &ctx_clone).await?;

        Ok(gas_sponsorship_info)
    }

    /// Potentially apply gas sponsorship to the given match response, returning
    /// the resulting `SponsoredMatchResponse`
    fn sponsor_direct_match_response(
        &self,
        ctx: &DirectMatchResponseCtx,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        let resp = ctx.response();
        let gas_sponsorship_info = ctx.sponsorship_info();
        if gas_sponsorship_info.is_none() {
            return Ok(SponsoredMatchResponse {
                match_bundle: resp.match_bundle,
                is_sponsored: false,
                gas_sponsorship_info: None,
            });
        }

        info!("Sponsoring match bundle via gas sponsor");
        let sponsorship_info = gas_sponsorship_info.unwrap();
        let sponsored_match_resp =
            self.construct_sponsored_match_response(resp, sponsorship_info)?;

        Ok(sponsored_match_resp)
    }

    // -----------
    // | Metrics |
    // -----------

    /// Record metrics for the direct match endpoint
    fn record_direct_match_metrics(&self, ctx: SponsoredDirectMatchResponseCtx) {
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.record_direct_match_metrics_helper(&ctx).await {
                warn!("Error handling direct match metrics: {e}");
            }
        });
    }

    /// A helper function to record metrics for the direct match endpoint
    async fn record_direct_match_metrics_helper(
        &self,
        ctx: &SponsoredDirectMatchResponseCtx,
    ) -> Result<(), AuthServerError> {
        // Because there is no price timestamp associated with a direct match,
        // we approximate it with the current time
        let price_timestamp = get_current_time_millis();
        // Record the bundle context in the store
        self.write_bundle_context(
            true, // shared
            price_timestamp,
            None, // assembled_timestamp
            ctx,
        )
        .await?;

        let req = ctx.request();
        let order = &req.external_order;
        self.handle_bundle_response(order, ctx)
    }
}
