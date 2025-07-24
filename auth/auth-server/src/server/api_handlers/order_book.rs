//! Orderbook endpoint handlers
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode};
use tracing::instrument;
use warp::reject::Rejection;

use super::{Server, log_unsuccessful_relayer_request};
use crate::server::api_handlers::external_match::BytesResponse;
use crate::telemetry::helpers::record_relayer_request_500;

impl Server {
    /// Proxy GET requests to /v0/order_book/* endpoints to the relayer
    #[instrument(skip(self, path, headers))]
    pub async fn handle_order_book_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
    ) -> Result<BytesResponse, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let (key_desc, _key_id) = self
            .authorize_request(path_str, "" /* query_str */, &headers, &[] /* body */)
            .await?;

        // Send the request to the relayer
        let resp =
            self.send_admin_request(Method::GET, path_str, headers.clone(), Bytes::new()).await?;

        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_desc.clone(), path_str.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&resp, &key_desc, path_str, &headers);
            return Ok(resp);
        }
        Ok(resp)
    }
}
