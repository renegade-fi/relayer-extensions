//! Server methods that watch for external match settlement

use super::Server;
use crate::bundle_store::BundleContext;
use crate::bundle_store::BundleId;
use crate::error::AuthServerError;
use crate::server::api_handlers::external_match::SponsoredExternalMatchResponseCtx;

/// The error message emitted when a nonce cannot be found
const ERR_NO_NONCE: &str = "No sponsorship nonce found";

impl Server {
    /// Write the bundle context to the store, handling gas sponsorship if
    /// necessary
    /// Returns the bundle ID
    pub fn write_bundle_context(
        &self,
        price_timestamp: u64,
        assembled_timestamp: Option<u64>,
        ctx: &SponsoredExternalMatchResponseCtx,
    ) -> Result<BundleId, AuthServerError> {
        // We use the gas sponsorship nonce as the bundle ID. This is a per-bundle
        // unique identifier that we can use to attribute settlement
        let bundle_id = ctx
            .sponsorship_nonce()
            .ok_or_else(|| AuthServerError::gas_sponsorship(ERR_NO_NONCE))?;

        // Create bundle context
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
            assembled_timestamp,
        };

        // Write to bundle store
        self.bundle_store.write(bundle_ctx);
        Ok(bundle_id)
    }
}
