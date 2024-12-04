//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use bytes::Bytes;
use http::Method;
use renegade_api::http::external_match::{ExternalMatchResponse, ExternalQuoteResponse};
use tracing::{info, instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::error::AuthServerError;

use super::Server;

/// Handle a proxied request
impl Server {
    /// Handle an external quote request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_quote_request(
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
        if let Err(e) = self.log_quote(resp.body()) {
            warn!("Error logging quote: {e}");
        }
        Ok(resp)
    }

    /// Handle an external quote-assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_external_quote_assembly_request(
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

    // --- Logging --- //

    /// Log a quote
    fn log_quote(&self, quote_bytes: &[u8]) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalQuoteResponse>(quote_bytes)
            .map_err(AuthServerError::serde)?;

        let match_result = resp.signed_quote.match_result();
        let is_buy = match_result.direction;
        let recv = resp.signed_quote.receive_amount();
        let send = resp.signed_quote.send_amount();
        info!(
            "Sending quote(is_buy: {is_buy}, receive: {} ({}), send: {} ({})) to client",
            recv.amount, recv.mint, send.amount, send.mint
        );

        Ok(())
    }

    /// Log the bundle parameters
    fn log_bundle(&self, bundle_bytes: &[u8]) -> Result<(), AuthServerError> {
        let resp = serde_json::from_slice::<ExternalMatchResponse>(bundle_bytes)
            .map_err(AuthServerError::serde)?;

        let match_result = resp.match_bundle.match_result;
        let is_buy = match_result.direction;
        let recv = resp.match_bundle.receive;
        let send = resp.match_bundle.send;
        info!(
            "Sending bundle(is_buy: {}, recv: {} ({}), send: {} ({})) to client",
            is_buy, recv.amount, recv.mint, send.amount, send.mint
        );

        Ok(())
    }
}
