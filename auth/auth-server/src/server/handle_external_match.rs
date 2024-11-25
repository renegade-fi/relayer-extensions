//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use bytes::Bytes;
use http::Method;
use renegade_api::http::external_match::ExternalMatchResponse;
use tracing::{info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::error::AuthServerError;

use super::Server;

/// Handle a proxied request
impl Server {
    /// Handle an external match request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_match_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        self.authorize_request(path.as_str(), &headers, &body).await?;

        // Send the request to the relayer
        let resp = self.send_admin_request(Method::POST, path.as_str(), headers, body).await?;

        // Log the bundle parameters
        if let Err(e) = self.log_bundle(resp.body()) {
            warn!("Error logging bundle: {e}");
        }
        Ok(resp)
    }

    /// Log the bundle parameters
    fn log_bundle(&self, bundle_bytes: &[u8]) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalMatchResponse>(bundle_bytes)
            .map_err(AuthServerError::serde)?;

        let match_result = resp.match_bundle.match_result;
        let is_buy = match_result.direction;
        let base_amount = match_result.base_amount;
        let quote_amount = match_result.quote_amount;
        info!(
            "Sending bundle(side: {}, base_amount: {}, quote_amount: {}) to client",
            is_buy, base_amount, quote_amount
        );

        Ok(())
    }
}
