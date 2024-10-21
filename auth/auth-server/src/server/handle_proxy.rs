//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use bytes::Bytes;
use http::Method;
use tracing::error;
use warp::{reject::Rejection, reply::Reply};

use crate::ApiError;

use super::Server;

/// Handle a proxied request
impl Server {
    /// Handle a request meant to be authenticated and proxied to the relayer
    pub async fn handle_proxy_request(
        &self,
        path: warp::path::FullPath,
        method: Method,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
    ) -> Result<impl Reply, Rejection> {
        let url = format!("{}{}", self.relayer_url, path.as_str());
        let req = self.client.request(method, &url).headers(headers).body(body);

        // TODO: Add admin auth here
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                let headers = resp.headers().clone();
                let body = resp.bytes().await.map_err(|e| {
                    warp::reject::custom(ApiError::InternalError(format!(
                        "Failed to read response body: {e}"
                    )))
                })?;

                let mut response = warp::http::Response::new(body);
                *response.status_mut() = status;
                *response.headers_mut() = headers;

                Ok(response)
            },
            Err(e) => {
                error!("Error proxying request: {}", e);
                Err(warp::reject::custom(ApiError::InternalError(e.to_string())))
            },
        }
    }
}
