//! RFQT Levels endpoint handler

use bytes::Bytes;
use http::{HeaderMap, Method};
use renegade_api::http::order_book::GET_DEPTH_FOR_ALL_PAIRS_ROUTE;
use tracing::instrument;
use warp::{reject::Rejection, reply::Json};

use crate::error::AuthServerError;
use crate::server::Server;
use crate::server::api_handlers::rfqt::helpers::{
    parse_levels_query_params, transform_depth_to_levels,
};

impl Server {
    /// Handle the RFQT Levels endpoint (`GET /rfqt/v3/levels`).
    #[instrument(skip(self, path, headers))]
    pub async fn handle_rfqt_levels_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        query_str: String,
    ) -> Result<Json, Rejection> {
        // Authorize request (path + query)
        let path_str = path.as_str();
        let (_key_desc, _key_id) =
            self.authorize_request(path_str, &query_str, &headers, &[] /* body */).await?;

        // Parse query params with validation
        let _params = parse_levels_query_params(&query_str, self.chain)?;

        // Send /v0/order_book_depth request to relayer
        let resp = self
            .send_admin_request(Method::GET, GET_DEPTH_FOR_ALL_PAIRS_ROUTE, headers, Bytes::new())
            .await?;

        // Deserialize and transform the response
        let depth_response = serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;
        let body = transform_depth_to_levels(depth_response);

        Ok(warp::reply::json(&body))
    }
}
