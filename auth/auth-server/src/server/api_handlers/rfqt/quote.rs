//! RFQT Quote endpoint handler

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Response, StatusCode, header::CONTENT_TYPE};
use tracing::instrument;
use warp::reject::Rejection;

use crate::error::AuthServerError;
use crate::server::Server;
use crate::server::api_handlers::external_match::BytesResponse;
use crate::server::api_handlers::rfqt::helpers::{dummy_quote_response, parse_quote_request};

impl Server {
    /// Handle the RFQT Quote endpoint (`POST /rfqt/v3/quote`).
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_rfqt_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<BytesResponse, Rejection> {
        // Authorize request
        let path_str = path.as_str();
        let (_key_desc, _key_id) =
            self.authorize_request(path_str, "" /* query_str */, &headers, &body).await?;

        // Parse 0x-zid header
        let _zid = headers
            .get("0x-zid")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AuthServerError::bad_request("Missing 0x-zid header"))?;

        // Parse request body
        let _request = parse_quote_request(&body)?;

        // Return dummy response with correct shape
        let response = dummy_quote_response();

        let bytes = Bytes::from(serde_json::to_vec(&response).map_err(AuthServerError::serde)?);

        let mut resp: Response<Bytes> = Response::new(bytes);
        *resp.status_mut() = StatusCode::OK;
        resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(resp)
    }
}
