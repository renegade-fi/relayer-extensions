//! Match bundle assembly endpoint handler

use auth_server_api::{GasSponsorshipInfo, SponsoredMatchResponse};
use bytes::Bytes;
use renegade_constants::GLOBAL_MATCHING_POOL;
use renegade_external_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalMatchAssemblyType, ExternalMatchResponse,
};
use renegade_external_api::types::{ApiSignedQuote, ExternalOrder};
use renegade_types_core::Token;
use renegade_util::get_current_time_millis;
use tracing::{error, info, instrument, warn};
use warp::reject::Rejection;

use crate::error::AuthServerError;
use crate::http_utils::request_response::overwrite_response_body;
use crate::server::api_handlers::external_match::BytesResponse;
use crate::server::gas_sponsorship::refund_calculation::{
    apply_gas_sponsorship_to_exact_output_amount, remove_gas_sponsorship_from_quote,
    requires_exact_output_amount_update,
};
use crate::server::helpers::generate_quote_uuid;
use crate::server::{
    Server,
    api_handlers::external_match::{ExternalMatchRequestType, RequestContext, ResponseContext},
};

// -----------------
// | Context Types |
// -----------------

/// The request context for a match bundle assembly request
type AssembleMatchRequestCtx = RequestContext<AssembleExternalMatchRequest>;

impl ExternalMatchRequestType for AssembleExternalMatchRequest {
    fn input_token(&self) -> Token {
        let input_mint = &self.order.get_external_order_ref().input_mint;
        Token::from_alloy_address(input_mint)
    }

    fn output_token(&self) -> Token {
        let output_mint = &self.order.get_external_order_ref().output_mint;
        Token::from_alloy_address(output_mint)
    }

    fn set_fee(&mut self, fee: f64) {
        self.options.relayer_fee_rate = Some(fee);
    }
}

/// The response context for an external match response
type ExternalMatchResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, ExternalMatchResponse>;
/// The response type for a sponsored external match response
pub type SponsoredExternalMatchResponseCtx =
    ResponseContext<AssembleExternalMatchRequest, SponsoredMatchResponse>;

impl SponsoredExternalMatchResponseCtx {
    /// Create a new sponsored external match response context from an external
    /// match response context
    pub fn from_external_match_response_ctx(
        sponsored_resp: SponsoredMatchResponse,
        ctx: ExternalMatchResponseCtx,
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

// --------------------
// | Endpoint Handler |
// --------------------

impl Server {
    /// Handle an external match bundle assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_assemble_match_bundle_request(
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

    /// Run the pre-request subroutines for the assembly endpoint
    #[instrument(skip_all)]
    async fn assembly_pre_request(
        &self,
        ctx: &mut AssembleMatchRequestCtx,
    ) -> Result<(), AuthServerError> {
        let key = ctx.key_id();
        let user = &ctx.user();
        if self.consume_bundle_rate_limit_token(key, user).await.is_err() {
            return Err(AuthServerError::no_match_found());
        };
        self.route_assembly_req(ctx).await?;

        // Apply gas sponsorship to the assembly request
        let gas_sponsorship_info = self.sponsor_assembly_request(ctx).await?;
        ctx.set_sponsorship_info(gas_sponsorship_info);
        Ok(())
    }

    /// Run the post-request subroutines for the assembly endpoint
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    fn assembly_post_request(
        &self,
        mut resp: BytesResponse,
        ctx: ExternalMatchResponseCtx,
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
        let ctx = SponsoredExternalMatchResponseCtx::from_external_match_response_ctx(
            sponsored_resp,
            ctx,
        );
        self.record_assembly_metrics(ctx);
        Ok(resp)
    }

    /// Route the assembly request to the correct matching pool
    ///
    /// If execution costs limits have been exceeded by the quoters, we route
    /// to the global pool to take pressure off them
    async fn route_assembly_req(
        &self,
        ctx: &mut AssembleMatchRequestCtx,
    ) -> Result<(), AuthServerError> {
        let ticker = ctx.body.base_ticker()?;
        let should_route_to_global = self.should_route_to_global(ctx.key_id(), &ticker).await?;
        if should_route_to_global {
            info!("Routing order to global matching pool");
            ctx.body_mut().options.matching_pool = Some(GLOBAL_MATCHING_POOL.to_string());
        }

        Ok(())
    }

    // -------------------
    // | Gas Sponsorship |
    // -------------------

    /// Check if the given assembly request pertains to a sponsored quote,
    /// and if so, remove the effects of gas sponsorship from the signed quote,
    /// and ensure sponsorship is correctly applied to the updated order, if
    /// present.
    ///
    /// We use the gas sponsorship nonce to track bundle attribution, so we
    /// always return a `GasSponsorshipInfo` instance, even if the trade is
    /// not sponsored.
    ///
    /// Returns the assembly request, and the gas sponsorship info (possibly
    /// zero)
    #[instrument(skip_all)]
    async fn sponsor_assembly_request(
        &self,
        ctx: &mut AssembleMatchRequestCtx,
    ) -> Result<GasSponsorshipInfo, AuthServerError> {
        let ctx_clone = ctx.clone();
        let req = ctx.body_mut();

        match &mut req.order {
            ExternalMatchAssemblyType::QuotedOrder { signed_quote, updated_order } => {
                self.sponsor_quoted_order(signed_quote, updated_order).await
            },
            ExternalMatchAssemblyType::DirectOrder { external_order } => {
                self.sponsor_direct_order(&ctx_clone, external_order).await
            },
        }
    }

    /// Apply gas sponsorship to a quoted order
    async fn sponsor_quoted_order(
        &self,
        signed_quote: &mut ApiSignedQuote,
        updated_order: &mut Option<ExternalOrder>,
    ) -> Result<GasSponsorshipInfo, AuthServerError> {
        let redis_key = generate_quote_uuid(signed_quote);
        let cached_info = match self.read_sponsorship_info_from_redis(redis_key).await {
            Err(e) => {
                error!("Error reading gas sponsorship info from Redis: {e}");
                None
            },
            Ok(cached_info) => cached_info,
        };

        if let Some(ref cached_info) = cached_info {
            // Reconstruct original signed quote with the cached original price
            if cached_info.gas_sponsorship_info.requires_match_result_update() {
                let quote = &mut signed_quote.quote;
                remove_gas_sponsorship_from_quote(quote, cached_info)?;
            }

            // Ensure that the exact output amount is respected on the updated order
            let gas_sponsorship_info = &cached_info.gas_sponsorship_info;
            if let Some(updated_order) = updated_order
                && requires_exact_output_amount_update(updated_order, gas_sponsorship_info)
            {
                apply_gas_sponsorship_to_exact_output_amount(updated_order, gas_sponsorship_info)?;
            }
        }

        // Return a zerod gas refund if no info was found
        let info =
            cached_info.map(|c| c.gas_sponsorship_info).unwrap_or_else(GasSponsorshipInfo::zero);
        Ok(info)
    }

    /// Apply gas sponsorship to a direct order
    async fn sponsor_direct_order(
        &self,
        ctx: &AssembleMatchRequestCtx,
        external_order: &mut ExternalOrder,
    ) -> Result<GasSponsorshipInfo, AuthServerError> {
        let gas_sponsorship_info = self.maybe_sponsor_order(external_order, ctx).await?;

        Ok(gas_sponsorship_info)
    }

    /// Potentially apply gas sponsorship to the given
    /// external match response, returning the resulting
    /// `SponsoredMatchResponse`
    fn sponsor_match_response(
        &self,
        ctx: &ExternalMatchResponseCtx,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        let resp = ctx.response();
        let gas_sponsorship_info = ctx.sponsorship_info_with_nonce();
        if gas_sponsorship_info.is_none() {
            return Ok(SponsoredMatchResponse {
                match_bundle: resp.match_bundle,
                gas_sponsorship_info: None,
            });
        }

        info!("Sponsoring match bundle via gas sponsor");
        let (sponsorship_info, nonce) = gas_sponsorship_info.unwrap();
        let sponsored_match_resp =
            self.construct_sponsored_match_response(resp, sponsorship_info, nonce)?;

        Ok(sponsored_match_resp)
    }

    // -----------
    // | Metrics |
    // -----------

    /// Record metrics for the assembly endpoint
    fn record_assembly_metrics(&self, ctx: SponsoredExternalMatchResponseCtx) {
        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.record_assembly_metrics_helper(&ctx) {
                warn!("Error handling assemble metrics: {e}");
            }
        });
    }

    /// A helper function to record metrics for the assembly endpoint
    fn record_assembly_metrics_helper(
        &self,
        ctx: &SponsoredExternalMatchResponseCtx,
    ) -> Result<(), AuthServerError> {
        match &ctx.request().order {
            ExternalMatchAssemblyType::QuotedOrder { signed_quote, updated_order } => {
                self.record_quoted_order_metrics(ctx, signed_quote, updated_order)
            },
            ExternalMatchAssemblyType::DirectOrder { external_order } => {
                self.record_direct_order_metrics(ctx, external_order)
            },
        }
    }

    /// Record metrics for a quoted order assembly
    fn record_quoted_order_metrics(
        &self,
        ctx: &SponsoredExternalMatchResponseCtx,
        signed_quote: &ApiSignedQuote,
        updated_order: &Option<ExternalOrder>,
    ) -> Result<(), AuthServerError> {
        let price_timestamp = signed_quote.quote.price.timestamp;
        let assembled_timestamp = get_current_time_millis();
        self.write_bundle_context(price_timestamp, Some(assembled_timestamp), ctx)?;

        let order = if let Some(updated_order) = updated_order {
            log_updated_order(ctx, signed_quote, updated_order);
            updated_order
        } else {
            &signed_quote.quote.order
        };

        self.handle_bundle_response(order, ctx)
    }

    /// Record metrics for a direct order assembly
    fn record_direct_order_metrics(
        &self,
        ctx: &SponsoredExternalMatchResponseCtx,
        external_order: &ExternalOrder,
    ) -> Result<(), AuthServerError> {
        let price_timestamp = get_current_time_millis();
        self.write_bundle_context(price_timestamp, None, ctx)?;
        self.handle_bundle_response(external_order, ctx)
    }
}

// -------------------
// | Logging Helpers |
// -------------------

/// Log an updated order
fn log_updated_order(
    ctx: &SponsoredExternalMatchResponseCtx,
    signed_quote: &ApiSignedQuote,
    updated_order: &ExternalOrder,
) {
    let original_order = &signed_quote.quote.order;

    let key = ctx.user();
    let request_id = ctx.request_id.to_string();
    let sdk_version = &ctx.sdk_version;

    let original_input_amount = original_order.input_amount;
    let updated_input_amount = updated_order.input_amount;
    let original_output_amount = original_order.output_amount;
    let updated_output_amount = updated_order.output_amount;

    info!(
        key_description = key,
        request_id = request_id,
        sdk_version = sdk_version,
        "Quote updated(original_input_amount: {}, updated_input_amount: {}, original_output_amount: {}, updated_output_amount: {})",
        original_input_amount,
        updated_input_amount,
        original_output_amount,
        updated_output_amount
    );
}
