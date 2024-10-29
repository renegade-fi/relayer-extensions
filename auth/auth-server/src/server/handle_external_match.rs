//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use bytes::Bytes;
use http::Method;
use warp::{reject::Rejection, reply::Reply};

use super::Server;

/// Handle a proxied request
impl Server {
    /// Handle an external match request
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
        Ok(resp)
    }
}
