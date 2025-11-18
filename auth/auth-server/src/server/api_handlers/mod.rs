//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

mod connectors;
mod external_match;
mod external_match_fees;
mod key_management;
mod order_book;
mod settlement;

use auth_server_api::{GasSponsorshipInfo, GasSponsorshipQueryParams, SponsoredMatchResponse};
use bytes::Bytes;
use external_match::{RequestContext, ResponseContext};
use http::{HeaderMap, Response};
use num_bigint::BigUint;
use rand::Rng;
use renegade_api::http::external_match::ExternalOrder;
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_common::types::token::Token;
use renegade_constants::{DEFAULT_EXTERNAL_MATCH_RELAYER_FEE, NATIVE_ASSET_WRAPPER_TICKER};
use renegade_util::hex::biguint_to_hex_addr;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use super::Server;
use super::gas_sponsorship::refund_calculation::{
    apply_gas_sponsorship_to_exact_output_amount, requires_exact_output_amount_update,
};
use crate::error::AuthServerError;
use crate::telemetry::helpers::calculate_implied_price;
use crate::telemetry::labels::{GAS_SPONSORED_METRIC_TAG, SDK_VERSION_METRIC_TAG};
use crate::telemetry::{
    helpers::record_external_match_metrics,
    labels::{KEY_DESCRIPTION_METRIC_TAG, REQUEST_ID_METRIC_TAG, REQUEST_PATH_METRIC_TAG},
};

/// The header name for the SDK version
const SDK_VERSION_HEADER: &str = "x-renegade-sdk-version";
/// The default SDK version to use if the header is not set
const SDK_VERSION_DEFAULT: &str = "pre-v0.1.0";
/// The name of the matching pool to route to if the execution cost rate limit
/// is exceeded
const GLOBAL_MATCHING_POOL: &str = "global";

/// A type alias for the response context for endpoints that return a match
/// bundle
///
/// This type is generic over request type, which allows us to use the same
/// handlers for endpoints with different request types but the same fundamental
/// match response type
type MatchBundleResponseCtx<Req> = ResponseContext<Req, SponsoredMatchResponse>;

/// Parse the SDK version from the given headers.
/// If unset or malformed, returns an empty string.
pub fn get_sdk_version(headers: &HeaderMap) -> String {
    headers
        .get(SDK_VERSION_HEADER)
        .map(|v| v.to_str().unwrap_or_default())
        .unwrap_or(SDK_VERSION_DEFAULT)
        .to_string()
}

/// Get a ticker from a `BigUint` encoded mint
pub fn ticker_from_biguint(mint: &BigUint) -> Result<String, AuthServerError> {
    let token = Token::from_addr_biguint(mint);
    if token.is_native_asset() {
        return Ok(NATIVE_ASSET_WRAPPER_TICKER.to_string());
    }

    token.get_ticker().ok_or_else(|| {
        let token_addr = biguint_to_hex_addr(mint);
        AuthServerError::bad_request(format!("Invalid token: {token_addr}"))
    })
}

// ---------------
// | Server Impl |
// ---------------

// General purpose methods useful to handlers defined in this module
impl Server {
    /// Determines if the current request should be sampled for metrics
    /// collection
    pub fn should_sample_metrics(&self) -> bool {
        rand::thread_rng().gen_bool(self.metrics_sampling_rate)
    }

    // --- Rate Limiting --- //

    /// Decide whether to route the given request to the global matching pool
    /// based on the per-asset rate limit and whitelist status of the key
    ///
    /// Routing to the global pool is equivalent to rate limiting in this
    /// context, because it removes the bot server's orders from
    /// consideration as counterparties, and instead requires that
    /// counterparties come from the global pool -- i.e. non-bot orders.
    ///
    /// An order is routed to the global pool if the bot server has exceeded its
    /// swap rate limit for the day. The exception is if the key is whitelisted,
    /// in which case it is never rate limited (routed to the global pool).
    pub async fn should_route_to_global(
        &self,
        key_id: Uuid,
        ticker: &str,
    ) -> Result<bool, AuthServerError> {
        let limit_exceeded = self.check_execution_cost_exceeded(ticker).await;
        let whitelisted = self.is_rate_limit_whitelisted(key_id).await?;
        let should_route_to_global = limit_exceeded && !whitelisted;
        Ok(should_route_to_global)
    }

    /// Check whether the given key is whitelisted to bypass external match flow
    /// rate limits
    pub async fn is_rate_limit_whitelisted(&self, key_id: Uuid) -> Result<bool, AuthServerError> {
        let key = self.get_api_key_entry(key_id).await?;
        Ok(key.rate_limit_whitelisted)
    }

    // --- Sponsorship --- //

    /// Generate gas sponsorship info for the given order if the query params
    /// call for it, and update the exact output amount requested in the order
    /// if necessary
    async fn maybe_sponsor_order<Req>(
        &self,
        order: &mut ExternalOrder,
        ctx: &RequestContext<Req>,
    ) -> Result<GasSponsorshipInfo, AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
    {
        // Parse query params
        let query = ctx.query();
        let query_params = serde_urlencoded::from_str::<GasSponsorshipQueryParams>(&query)
            .map_err(AuthServerError::serde)?;

        // Generate gas sponsorship info
        let user = ctx.user();
        let gas_sponsorship_info =
            self.generate_sponsorship_info(&user, order, &query_params).await?;

        // Subtract the refund amount from the exact output amount requested in the
        // order, so that the relayer produces a smaller quote which will
        // match the exact output amount after the refund is issued
        if requires_exact_output_amount_update(order, &gas_sponsorship_info) {
            info!(
                "Adjusting exact output amount requested in order to account for gas sponsorship"
            );
            apply_gas_sponsorship_to_exact_output_amount(order, &gas_sponsorship_info);
        }

        Ok(gas_sponsorship_info)
    }

    // --- Bundle Tracking --- //

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    #[allow(clippy::too_many_arguments)]
    fn handle_bundle_response<Req>(
        &self,
        order: &ExternalOrder,
        ctx: &MatchBundleResponseCtx<Req>,
    ) -> Result<(), AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
    {
        // Log the bundle
        log_bundle(order, ctx)?;

        // Note: if sponsored in-kind w/ refund going to the receiver,
        // the amounts in the match bundle will have been updated
        let SponsoredMatchResponse { match_bundle, is_sponsored, .. } = ctx.response();

        let labels = vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.user()),
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.to_string()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), is_sponsored.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), ctx.sdk_version.clone()),
            (REQUEST_PATH_METRIC_TAG.to_string(), ctx.path.clone()),
        ];

        // Record metrics
        record_external_match_metrics(order, &match_bundle, &labels)?;
        Ok(())
    }
}

// -------------------
// | Logging helpers |
// -------------------

/// Log a non-200 response from the relayer for the given request
pub fn log_unsuccessful_relayer_request(
    resp: &Response<Bytes>,
    key_description: &str,
    path: &str,
    headers: &HeaderMap,
) {
    let status = resp.status();
    let text = String::from_utf8_lossy(resp.body()).to_string();
    let sdk_version = get_sdk_version(headers);
    warn!(
        key_description = key_description,
        path = path,
        sdk_version = sdk_version,
        "Non-200 response from relayer: {status}: {text}",
    );
}

/// Log the bundle parameters
fn log_bundle<Req>(
    order: &ExternalOrder,
    ctx: &MatchBundleResponseCtx<Req>,
) -> Result<(), AuthServerError>
where
    Req: Serialize + for<'de> Deserialize<'de>,
{
    let SponsoredMatchResponse { match_bundle, is_sponsored, gas_sponsorship_info } =
        ctx.response();

    // Get the decimal-corrected price
    let price = calculate_implied_price(&match_bundle, true /* decimal_correct */)?;
    let price_fixed = FixedPoint::from_f64_round_down(price);

    let match_result = &match_bundle.match_result;
    let is_buy = match_result.direction;
    let recv = &match_bundle.receive;
    let send = &match_bundle.send;

    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);

    // Get the base fill ratio
    let requested_base_amount = order.get_base_amount(price_fixed, relayer_fee);
    let response_base_amount = match_result.base_amount;
    let base_fill_ratio = response_base_amount as f64 / requested_base_amount as f64;

    // Get the quote fill ratio
    let requested_quote_amount = order.get_quote_amount(price_fixed, relayer_fee);
    let response_quote_amount = match_result.quote_amount;
    let quote_fill_ratio = response_quote_amount as f64 / requested_quote_amount as f64;

    // Get the gas sponsorship info
    let (refund_amount, refund_native_eth) = gas_sponsorship_info
        .as_ref()
        .map(|info| (info.refund_amount, info.refund_native_eth))
        .unwrap_or((0, false));

    let key_description = ctx.user();
    let request_id = ctx.request_id.to_string();
    info!(
        requested_base_amount = requested_base_amount,
        response_base_amount = response_base_amount,
        requested_quote_amount = requested_quote_amount,
        response_quote_amount = response_quote_amount,
        base_fill_ratio = base_fill_ratio,
        quote_fill_ratio = quote_fill_ratio,
        key_description = key_description,
        request_id = request_id,
        is_sponsored = is_sponsored,
        endpoint = ctx.path,
        sdk_version = ctx.sdk_version,
        "Sending bundle(is_buy: {}, recv: {} ({}), send: {} ({}), refund_amount: {} (refund_native_eth: {})) to client",
        is_buy,
        recv.amount,
        recv.mint,
        send.amount,
        send.mint,
        refund_amount,
        refund_native_eth
    );

    Ok(())
}
