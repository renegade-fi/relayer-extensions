//! RFQT Levels endpoint handler

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Response, StatusCode, header::CONTENT_TYPE};
use tracing::instrument;
use warp::reject::Rejection;

use super::types::{RfqtLevelsQueryParams, dummy_levels_body, parse_levels_query_params};
use crate::error::AuthServerError;
use crate::server::Server;
use crate::server::api_handlers::external_match::BytesResponse;

impl Server {
    /// Handle the RFQT Levels endpoint (`GET /rfqt/v3/levels`).
    #[instrument(skip(self, path, headers))]
    pub async fn handle_rfqt_levels_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        query_str: String,
    ) -> Result<BytesResponse, Rejection> {
        // Authorize request (path + query)
        let path_str = path.as_str();
        let (_key_desc, _key_id) =
            self.authorize_request(path_str, &query_str, &headers, &[] /* body */).await?;

        // Parse query params
        let _params: RfqtLevelsQueryParams = parse_levels_query_params(&query_str);

        // Return dummy response with correct shape
        let body = dummy_levels_body();

        let bytes = Bytes::from(serde_json::to_vec(&body).map_err(AuthServerError::serde)?);

        let mut resp: Response<Bytes> = Response::new(bytes);
        *resp.status_mut() = StatusCode::OK;
        resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(resp)
    }
}
