//! Assemble quote endpoint handler

use auth_server_api::{GasSponsorshipInfo, SponsoredMatchResponse};
use bytes::Bytes;
use num_bigint::BigUint;
use renegade_api::http::external_match::{AssembleExternalMatchRequest, ExternalMatchResponse};
use renegade_util::get_current_time_millis;
use tracing::{error, info, instrument, warn};
use warp::reject::Rejection;

use crate::{
    error::AuthServerError,
    http_utils::request_response::overwrite_response_body,
    server::{
        Server,
        api_handlers::{
            GLOBAL_MATCHING_POOL,
            external_match::{BytesResponse, ExternalMatchRequestType},
            ticker_from_biguint,
        },
        gas_sponsorship::refund_calculation::{
            apply_gas_sponsorship_to_exact_output_amount, remove_gas_sponsorship_from_quote,
            requires_exact_output_amount_update,
        },
        helpers::generate_quote_uuid,
    },
};

use super::{RequestContext, ResponseContext};

// -----------------
// | Context Types |
// -----------------

/// The request context for an assemble quote request
pub(crate) type AssembleQuoteRequestCtx = RequestContext<AssembleExternalMatchRequest>;
impl AssembleQuoteRequestCtx {
    /// Get the ticker from the request
    pub fn ticker(&self) -> Result<String, AuthServerError> {
        ticker_from_biguint(&self.body.signed_quote.quote.order.base_mint)
    }
}

impl ExternalMatchRequestType for AssembleExternalMatchRequest {
    fn base_mint(&self) -> &BigUint {
        &self.signed_quote.quote.order.base_mint
    }

    fn quote_mint(&self) -> &BigUint {
        &self.signed_quote.quote.order.quote_mint
    }

    fn set_fee(&mut self, fee: f64) {
        self.relayer_fee_rate = fee;
    }
}

/// The response context for an assemble quote request
type AssembleQuoteResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, ExternalMatchResponse>;
/// The response context for a sponsored assemble quote response
type SponsoredAssembleQuoteResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, SponsoredMatchResponse>;

impl SponsoredAssembleQuoteResponseCtx {
    /// Create a new sponsored assemble quote response context
    pub fn from_assemble_quote_response_ctx(
        sponsored_resp: SponsoredMatchResponse,
        ctx: AssembleQuoteResponseCtx,
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
    /// Handle an external quote-assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_assemble_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<BytesResponse, Rejection> {
        // 1. Run the pre-request subroutines
        let mut ctx = self.preprocess_request(path, headers, body, query_str).await?;
        self.assembly_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.assembly_post_request(raw_resp, ctx)?;
        Ok(res)
    }

    // -------------------------------
    // | Request Pre/Post Processing |
    // -------------------------------

    /// Run the pre-request subroutines for the assembly quote endpoint
    #[instrument(skip_all)]
    async fn assembly_pre_request(
        &self,
        ctx: &mut AssembleQuoteRequestCtx,
    ) -> Result<(), AuthServerError> {
        let allow_shared = ctx.body.allow_shared;
        let key_desc = ctx.user();
        self.check_bundle_rate_limit(key_desc, allow_shared).await?;

        // Apply gas sponsorship to the assembly request
        let gas_sponsorship_info = self.sponsor_assembly_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run the post-request subroutines for the assembly quote endpoint
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    fn assembly_post_request(
        &self,
        mut resp: BytesResponse,
        ctx: AssembleQuoteResponseCtx,
    ) -> Result<BytesResponse, AuthServerError> {
        // If the relayer returns non-200, return the response directly
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Apply gas sponsorship to the resulting bundle, if necessary
        let sponsored_resp = self.sponsor_match_response(&ctx)?;
        let should_stringify = ctx.should_stringify_body();
        overwrite_response_body(&mut resp, sponsored_resp.clone(), should_stringify)?;

        // Record metrics
        let ctx = SponsoredAssembleQuoteResponseCtx::from_assemble_quote_response_ctx(
            sponsored_resp,
            ctx,
        );
        self.record_assemble_metrics(ctx);
        Ok(resp)
    }

    // -------------------
    // | Gas Sponsorship |
    // -------------------

    /// Route the assembly request to the correct matching pool
    ///
    /// If execution costs limits have been exceeded by the bot server, we route
    /// to the global pool to take pressure off the quoters
    pub(crate) async fn route_assembly_req(
        &self,
        ctx: &mut AssembleQuoteRequestCtx,
    ) -> Result<(), AuthServerError> {
        let ticker = ctx.ticker()?;
        let should_route_to_global = self.should_route_to_global(ctx.key_id(), &ticker).await?;
        if should_route_to_global {
            info!("Routing order to global matching pool");
            ctx.body_mut().matching_pool = Some(GLOBAL_MATCHING_POOL.to_string());
        }

        Ok(())
    }

    /// Check if the given assembly request pertains to a sponsored quote,
    /// and if so, remove the effects of gas sponsorship from the signed quote,
    /// and ensure sponsorship is correctly applied to the updated order, if
    /// present.
    ///
    /// Returns the assembly request, and the gas sponsorship info, if any.
    #[instrument(skip_all)]
    pub(crate) async fn sponsor_assembly_request(
        &self,
        ctx: &mut AssembleQuoteRequestCtx,
    ) -> Result<Option<GasSponsorshipInfo>, AuthServerError> {
        let req = ctx.body_mut();
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

    /// Potentially apply gas sponsorship to the given
    /// external match response, returning the resulting
    /// `SponsoredMatchResponse`
    fn sponsor_match_response(
        &self,
        ctx: &AssembleQuoteResponseCtx,
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

    /// Record metrics for the assemble quote endpoint
    fn record_assemble_metrics(&self, ctx: SponsoredAssembleQuoteResponseCtx) {
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.record_assemble_metrics_helper(&ctx).await {
                warn!("Error handling assemble metrics: {e}");
            }
        });
    }

    /// A helper function to record metrics for the assemble quote endpoint
    async fn record_assemble_metrics_helper(
        &self,
        ctx: &SponsoredAssembleQuoteResponseCtx,
    ) -> Result<(), AuthServerError> {
        // Record the bundle context in the store
        let shared = ctx.request().allow_shared;
        let price_timestamp = ctx.request().signed_quote.quote.price.timestamp;
        let assembled_timestamp = get_current_time_millis();
        self.write_bundle_context(shared, price_timestamp, Some(assembled_timestamp), ctx).await?;

        let req = ctx.request();
        if req.updated_order.is_some() {
            log_updated_order(ctx);
        }

        let order = &req.signed_quote.quote.order;
        self.handle_bundle_response(order, ctx)
    }
}

// -------------------
// | Logging Helpers |
// -------------------

/// Log an updated order
fn log_updated_order(ctx: &SponsoredAssembleQuoteResponseCtx) {
    let req = ctx.request();
    let original_order = &req.signed_quote.quote.order;
    let updated_order = req.updated_order.as_ref().unwrap_or(original_order);

    let key = ctx.user();
    let request_id = ctx.request_id.to_string();
    let sdk_version = &ctx.sdk_version;

    let original_base_amount = original_order.base_amount;
    let updated_base_amount = updated_order.base_amount;
    let original_quote_amount = original_order.quote_amount;
    let updated_quote_amount = updated_order.quote_amount;
    info!(
        key_description = key,
        request_id = request_id,
        sdk_version = sdk_version,
        "Quote updated(original_base_amount: {}, updated_base_amount: {}, original_quote_amount: {}, updated_quote_amount: {})",
        original_base_amount,
        updated_base_amount,
        original_quote_amount,
        updated_quote_amount
    );
}
