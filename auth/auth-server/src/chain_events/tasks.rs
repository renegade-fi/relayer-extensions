use renegade_api::http::external_match::ApiExternalMatchResult;

use crate::telemetry::{
    helpers::{extend_labels_with_base_asset, record_endpoint_metrics, record_volume_with_tags},
    labels::{
        EXTERNAL_MATCH_SETTLED_BASE_VOLUME, EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
        GAS_SPONSORED_METRIC_TAG, KEY_DESCRIPTION_METRIC_TAG, NUM_EXTERNAL_MATCH_REQUESTS,
        REQUEST_ID_METRIC_TAG, SDK_VERSION_METRIC_TAG, SETTLEMENT_STATUS_TAG,
    },
};
use crate::{chain_events::listener::OnChainEventListenerExecutor, store::BundleContext};

impl OnChainEventListenerExecutor {
    /// Record settlement metrics for a bundle
    pub fn record_settlement_metrics(
        &self,
        ctx: &BundleContext,
        match_result: &ApiExternalMatchResult,
    ) {
        let labels = self.get_labels(ctx);
        record_endpoint_metrics(&match_result.base_mint, NUM_EXTERNAL_MATCH_REQUESTS, &labels);

        record_volume_with_tags(
            &match_result.base_mint,
            match_result.base_amount,
            EXTERNAL_MATCH_SETTLED_BASE_VOLUME,
            &labels,
        );

        let labels = extend_labels_with_base_asset(&match_result.base_mint, labels);
        record_volume_with_tags(
            &match_result.quote_mint,
            match_result.quote_amount,
            EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME,
            &labels,
        );
    }

    /// Increment the token balance for a given API user
    pub async fn add_bundle_rate_limit_token(&self, key_description: String, shared: bool) {
        self.rate_limiter.add_bundle_token(key_description, shared).await;
    }

    /// Get the labels for a bundle
    fn get_labels(&self, ctx: &BundleContext) -> Vec<(String, String)> {
        vec![
            (KEY_DESCRIPTION_METRIC_TAG.to_string(), ctx.key_description.clone()),
            (REQUEST_ID_METRIC_TAG.to_string(), ctx.request_id.clone()),
            (GAS_SPONSORED_METRIC_TAG.to_string(), ctx.is_sponsored.to_string()),
            (SDK_VERSION_METRIC_TAG.to_string(), ctx.sdk_version.clone()),
            (SETTLEMENT_STATUS_TAG.to_string(), "true".to_string()),
        ]
    }
}
