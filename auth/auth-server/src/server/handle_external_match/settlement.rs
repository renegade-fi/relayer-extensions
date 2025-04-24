use auth_server_api::{GasSponsorshipInfo, SponsoredMatchResponse}; // Added GasSponsorshipInfo
use http::HeaderMap;
use renegade_api::http::external_match::ApiExternalMatchResult;
use renegade_circuit_types::order::OrderSide;

use super::{get_sdk_version, Server};
use crate::error::AuthServerError;
use crate::store::{helpers::generate_bundle_id, BundleContext};
use crate::telemetry::helpers::extract_nullifier_from_match_bundle;

impl Server {
    /// Write the bundle context to the store, handling gas sponsorship if
    /// necessary
    /// Returns the bundle ID
    pub async fn write_bundle_context(
        &self,
        sponsored_match: &SponsoredMatchResponse,
        headers: &HeaderMap,
        key: String,
        shared: bool,
    ) -> Result<String, AuthServerError> {
        // Extract the nullifier from the original match bundle
        let nullifier = extract_nullifier_from_match_bundle(&sponsored_match.match_bundle)?;

        // Determine the match result to derive ID, accounting for sponsorship
        let match_result_for_id = self.get_match_result_for_id(sponsored_match);

        // Generate bundle ID using the potentially adjusted match result
        let bundle_id = generate_bundle_id(&match_result_for_id, &nullifier)?;

        // Create bundle context
        let bundle_ctx = BundleContext {
            key_description: key.clone(),
            request_id: bundle_id.clone(),
            sdk_version: get_sdk_version(headers),
            gas_sponsorship_info: sponsored_match.gas_sponsorship_info.clone(),
            is_sponsored: sponsored_match.is_sponsored,
            nullifier,
            shared,
        };

        // Write to bundle store
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
