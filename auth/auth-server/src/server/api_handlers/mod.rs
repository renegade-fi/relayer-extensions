//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

// mod connectors;
mod exchange_metadata;
mod external_match;
mod external_match_fees;
mod key_management;
mod markets;
mod settlement;

use auth_server_api::{GasSponsorshipInfo, GasSponsorshipQueryParams, SponsoredMatchResponse};
use bytes::Bytes;
use external_match::RequestContext;
use http::{HeaderMap, Response};
use rand::Rng;
use renegade_circuit_types::Amount;
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_constants::DEFAULT_EXTERNAL_MATCH_RELAYER_FEE;
use renegade_crypto::fields::scalar_to_u128;
use renegade_external_api::types::ExternalOrder;
use renegade_util::hex::address_to_hex_string;
use renegade_util::on_chain::get_protocol_fee;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

use super::Server;
use super::gas_sponsorship::refund_calculation::{
    apply_gas_sponsorship_to_exact_output_amount, requires_exact_output_amount_update,
};
use crate::error::AuthServerError;
pub use crate::server::api_handlers::external_match::SponsoredExternalMatchResponseCtx;
use crate::server::helpers::pick_base_and_quote_mints;
use crate::telemetry::helpers::{
    calculate_quote_per_base_price, get_default_base_amount, get_default_quote_amount,
};
use crate::telemetry::labels::{GAS_SPONSORED_METRIC_TAG, SDK_VERSION_METRIC_TAG};
use crate::telemetry::{
    helpers::record_external_match_metrics,
    labels::{KEY_DESCRIPTION_METRIC_TAG, REQUEST_ID_METRIC_TAG, REQUEST_PATH_METRIC_TAG},
};

/// The header name for the SDK version
const SDK_VERSION_HEADER: &str = "x-renegade-sdk-version";
/// The default SDK version to use if the header is not set
const SDK_VERSION_DEFAULT: &str = "pre-v0.1.0";
/// The default relayer fee to charge if no per-user or per-asset fee is set
pub(crate) const DEFAULT_RELAYER_FEE: f64 = 0.0001; // 1bp

/// Parse the SDK version from the given headers.
/// If unset or malformed, returns an empty string.
pub fn get_sdk_version(headers: &HeaderMap) -> String {
    headers
        .get(SDK_VERSION_HEADER)
        .map(|v| v.to_str().unwrap_or_default())
        .unwrap_or(SDK_VERSION_DEFAULT)
        .to_string()
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
            apply_gas_sponsorship_to_exact_output_amount(order, &gas_sponsorship_info)?;
        }

        Ok(gas_sponsorship_info)
    }

    // --- Bundle Tracking --- //

    /// Record and watch a bundle that was forwarded to the client
    ///
    /// This method will await settlement and update metrics, rate limits, etc
    fn handle_bundle_response(
        &self,
        order: &ExternalOrder,
        ctx: &SponsoredExternalMatchResponseCtx,
    ) -> Result<(), AuthServerError> {
        // Log the bundle
        log_bundle(order, ctx)?;

        // Note: if sponsored in-kind w/ refund going to the receiver,
        // the amounts in the match bundle will have been updated
        let SponsoredMatchResponse { match_bundle, gas_sponsorship_info, .. } = ctx.response();
        let is_sponsored = gas_sponsorship_info.is_some();

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

    // --- Helpers --- //

    /// Get the quote amount for the given order, fetching a price from the
    /// price reporter client if necessary
    pub(crate) async fn get_quote_amount(
        &self,
        order: &ExternalOrder,
        relayer_fee: FixedPoint,
    ) -> Result<Amount, AuthServerError> {
        let (base_mint, _) = pick_base_and_quote_mints(order.input_mint, order.output_mint)?;

        let price = self
            .price_reporter_client
            .get_price(&address_to_hex_string(&base_mint), self.chain)
            .await?;

        let (_, quote_amount) = get_base_and_quote_amount_with_price(order, relayer_fee, price)?;
        Ok(quote_amount)
    }
}

// ----------------------------
// | Quote Conversion Helpers |
// ----------------------------

/// Get the quote amount that will be used in matching this order.
/// Importantly, this method accounts for fees charged on the order
/// in the case that an exact quote output amount is requested.
pub(crate) fn get_base_and_quote_amount_with_price(
    order: &ExternalOrder,
    relayer_fee: FixedPoint,
    price: f64,
) -> Result<(Amount, Amount), AuthServerError> {
    let (base_mint, quote_mint) = pick_base_and_quote_mints(order.input_mint, order.output_mint)?;

    let base_input_set = base_mint == order.input_mint && order.input_amount != 0;
    let base_output_set = base_mint == order.output_mint && order.output_amount != 0;
    let quote_input_set = quote_mint == order.input_mint && order.input_amount != 0;

    let price_fp = FixedPoint::from_f64_round_down(price);

    if base_input_set {
        let base_amount = order.input_amount;
        let implied_quote_amount = price_fp * base_amount;
        let quote_amount = scalar_to_u128(&implied_quote_amount.floor());
        Ok((base_amount, quote_amount))
    } else if base_output_set {
        let base_amount = fee_adjusted_output_amount(order, relayer_fee)?;
        let implied_quote_amount = price_fp * base_amount;
        let quote_amount = scalar_to_u128(&implied_quote_amount.floor());
        Ok((base_amount, quote_amount))
    } else if quote_input_set {
        let quote_amount = order.input_amount;
        let implied_base_amount = price_fp.floor_div_int(quote_amount);
        let base_amount = scalar_to_u128(&implied_base_amount);
        Ok((base_amount, quote_amount))
    } else {
        let quote_amount = fee_adjusted_output_amount(order, relayer_fee)?;
        let implied_base_amount = price_fp.floor_div_int(quote_amount);
        let base_amount = scalar_to_u128(&implied_base_amount);
        Ok((base_amount, quote_amount))
    }
}
/// Calculate the output amount that will be used in matching this order,
/// accounting for fees charged on the order in the case that an exact
/// output amount is requested.
fn fee_adjusted_output_amount(
    order: &ExternalOrder,
    relayer_fee: FixedPoint,
) -> Result<Amount, AuthServerError> {
    let output_amount = order.output_amount;
    if !order.use_exact_output_amount {
        return Ok(output_amount);
    }

    let (base_mint, quote_mint) = pick_base_and_quote_mints(order.input_mint, order.output_mint)?;

    let protocol_fee = get_protocol_fee(&base_mint, &quote_mint);
    let total_fee = protocol_fee + relayer_fee;

    let one_minus_fee = FixedPoint::one() - total_fee;
    let adjusted_amount = one_minus_fee.floor_div_int(output_amount);
    Ok(scalar_to_u128(&adjusted_amount))
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
fn log_bundle(
    order: &ExternalOrder,
    ctx: &SponsoredExternalMatchResponseCtx,
) -> Result<(), AuthServerError> {
    let SponsoredMatchResponse { match_bundle, gas_sponsorship_info } = ctx.response();
    let is_sponsored = gas_sponsorship_info.is_some();

    // Get the decimal-corrected price
    let price = calculate_quote_per_base_price(&match_bundle.match_result)?;

    let match_result = &match_bundle.match_result;
    let (base_mint, _) =
        pick_base_and_quote_mints(match_result.input_mint, match_result.output_mint)?;
    let is_buy = base_mint == match_result.output_mint;
    let min_recv = &match_bundle.min_receive;
    let max_recv = &match_bundle.max_receive;
    let min_send = &match_bundle.min_send;
    let max_send = &match_bundle.max_send;

    let relayer_fee = FixedPoint::from_f64_round_down(DEFAULT_EXTERNAL_MATCH_RELAYER_FEE);
    let (requested_base_amount, requested_quote_amount) =
        get_base_and_quote_amount_with_price(order, relayer_fee, price)?;

    // Get the base fill ratio
    let response_base_amount = get_default_base_amount(&match_bundle)?;
    let base_fill_ratio = response_base_amount as f64 / requested_base_amount as f64;

    // Get the quote fill ratio
    let response_quote_amount = get_default_quote_amount(&match_bundle)?;
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
        "Sending bundle(is_buy: {}, recv: [{}, {}] ({}), send: [{}, {}] ({}), refund_amount: {} (refund_native_eth: {})) to client",
        is_buy,
        min_recv.amount,
        max_recv.amount,
        min_recv.mint,
        min_send.amount,
        max_send.amount,
        min_send.mint,
        refund_amount,
        refund_native_eth
    );

    Ok(())
}
