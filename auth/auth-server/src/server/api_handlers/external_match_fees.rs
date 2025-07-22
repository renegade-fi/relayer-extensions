//! Handles admin external match fee requests

use bytes::Bytes;
use http::HeaderMap;
use tracing::instrument;
use warp::{filters::path::FullPath, reject::Rejection, reply::Reply};

use crate::{http_utils::request_response::empty_json_reply, server::Server};

impl Server {
    // --- Getters --- //

    /// Get the per-asset, per-user fee for all users and assets
    #[instrument(skip_all)]
    pub async fn get_all_user_fees(
        &self,
        path: FullPath,
        headers: HeaderMap,
    ) -> Result<impl Reply, Rejection> {
        self.authorize_management_request(&path, &headers, &Bytes::new() /* body */)?;

        Ok(empty_json_reply())
    }

    // --- Setters --- //

    /// Set the default fee for a given asset
    #[instrument(skip_all)]
    pub async fn set_asset_default_fee(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        Ok(empty_json_reply())
    }

    /// Set the per-user fee override for a given asset
    #[instrument(skip_all)]
    pub async fn set_user_fee_override(
        &self,
        path: FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        // Check management auth on the request
        self.authorize_management_request(&path, &headers, &body)?;

        Ok(empty_json_reply())
    }
}
