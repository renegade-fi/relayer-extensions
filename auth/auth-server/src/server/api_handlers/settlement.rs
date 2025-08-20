//! Server methods that watch for external match settlement
use auth_server_api::{GasSponsorshipInfo, SponsoredMatchResponse}; // Added GasSponsorshipInfo
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_circuit_types::order::OrderSide;
use serde::{Deserialize, Serialize};

use super::{MatchBundleResponseCtx, Server};
use crate::bundle_store::helpers::generate_malleable_bundle_id;
use crate::bundle_store::{BundleContext, helpers::generate_bundle_id};
use crate::error::AuthServerError;
use crate::server::api_handlers::external_match::SponsoredAssembleMalleableQuoteResponseCtx;
use crate::telemetry::abi_helpers::{
    extract_nullifier_from_malleable_match_bundle, extract_nullifier_from_match_bundle,
};

impl Server {
    /// Write the bundle context to the store, handling gas sponsorship if
    /// necessary
    /// Returns the bundle ID
    pub async fn write_bundle_context<Req>(
        &self,
        price_timestamp: u64,
        assembled_timestamp: Option<u64>,
        ctx: &MatchBundleResponseCtx<Req>,
    ) -> Result<String, AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
    {
        let resp = ctx.response();
        // Extract the nullifier from the original match bundle
        let nullifier = extract_nullifier_from_match_bundle(&resp.match_bundle)?;

        // Determine the match result to derive ID, accounting for sponsorship
        let match_result_for_id = self.get_match_result_for_id(&resp);

        // Generate bundle ID using the potentially adjusted match result
        let bundle_id = generate_bundle_id(&match_result_for_id, &nullifier)?;

        // Create bundle context
        let gas_sponsorship_info = ctx.sponsorship_info_with_nonce();
        let is_sponsored = gas_sponsorship_info.is_some();
        let bundle_ctx = BundleContext {
            key_description: ctx.user(),
            request_id: bundle_id.clone(),
            sdk_version: ctx.sdk_version.clone(),
            gas_sponsorship_info,
            is_sponsored,
            nullifier,
            price_timestamp,
            assembled_timestamp,
        };

        // Write to bundle store
        if let Err(e) = self.bundle_store.write(bundle_id.clone(), bundle_ctx).await {
            tracing::error!("bundle context write failed: {}", e);
        }

        Ok(bundle_id)
    }

    /// Write the bundle context for a malleable match to the store
    pub async fn write_malleable_bundle_context(
        &self,
        assembled_timestamp: Option<u64>,
        ctx: &SponsoredAssembleMalleableQuoteResponseCtx,
    ) -> Result<String, AuthServerError> {
        let req = ctx.request();
        let resp = ctx.response();

        // Generate bundle ID and context
        let nullifier = extract_nullifier_from_malleable_match_bundle(&resp.match_bundle)?;
        let bundle_id = generate_malleable_bundle_id(&resp.match_bundle.match_result, &nullifier)?;

        let gas_sponsorship_info = ctx.sponsorship_info_with_nonce();
        let is_sponsored = gas_sponsorship_info.is_some();
        let price_timestamp = req.signed_quote.quote.price.timestamp;

        let bundle_ctx = BundleContext {
            key_description: ctx.user(),
            request_id: bundle_id.clone(),
            sdk_version: ctx.sdk_version.clone(),
            gas_sponsorship_info,
            is_sponsored,
            nullifier,
            price_timestamp,
            assembled_timestamp,
        };

        if let Err(e) = self.bundle_store.write(bundle_id.clone(), bundle_ctx).await {
            tracing::error!("bundle context write failed: {}", e);
        }

        Ok(bundle_id)
    }

    /// Determines the appropriate `ApiExternalMatchResult` to use for bundle ID
    /// generation. If the match is sponsored and info is present, it returns
    /// a result adjusted for the gas refund; otherwise, it returns the
    /// original match result.
    fn get_match_result_for_id(
        &self,
        sponsored_match: &SponsoredMatchResponse,
    ) -> ApiExternalMatchResult {
        if let Some(gas_info) = sponsored_match.gas_sponsorship_info.as_ref() {
            self.apply_gas_sponsorship_adjustment(
                &sponsored_match.match_bundle.match_result,
                gas_info,
            )
        } else {
            sponsored_match.match_bundle.match_result.clone()
        }
    }

    /// Returns a new match result with gas sponsorship amount subtracted from
    /// the appropriate side, if the refund is not native ETH.
    fn apply_gas_sponsorship_adjustment(
        &self,
        original_result: &ApiExternalMatchResult,
        gas_sponsorship_info: &GasSponsorshipInfo,
    ) -> ApiExternalMatchResult {
        let mut result = original_result.clone();

        // If the refund is in native ETH, no adjustment needed for the match result
        // amounts
        if gas_sponsorship_info.refund_native_eth {
            return result;
        }

        match result.direction {
            OrderSide::Buy => {
                result.base_amount -= gas_sponsorship_info.refund_amount;
            },
            OrderSide::Sell => {
                result.quote_amount -= gas_sponsorship_info.refund_amount;
            },
        }
        result
    }
}
