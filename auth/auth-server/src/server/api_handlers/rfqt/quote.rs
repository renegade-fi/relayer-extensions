//! RFQT Quote endpoint handler

use bytes::Bytes;
use http::HeaderMap;
use tracing::instrument;
use uuid::Uuid;
use warp::{reject::Rejection, reply::Json};

use renegade_api::http::external_match::{
    ExternalMatchRequest, ExternalMatchResponse, REQUEST_EXTERNAL_MATCH_ROUTE,
};

use auth_server_api::rfqt::RfqtQuoteRequest;

use crate::error::AuthServerError;
use crate::http_utils::stringify_formatter::json_deserialize;
use crate::server::Server;
use crate::server::api_handlers::external_match::{BytesResponse, RequestContext, ResponseContext};
use crate::server::api_handlers::get_sdk_version;
use crate::server::api_handlers::rfqt::helpers::{
    transform_external_match_to_rfqt_response, transform_rfqt_to_external_match_request,
};

impl Server {
    /// Handle the RFQT Quote endpoint (`POST /rfqt/v3/quote`).
    #[instrument(skip(self, headers, body))]
    pub async fn handle_rfqt_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<Json, Rejection> {
        // Parse 0x-zid header
        let _zid = headers
            .get("0x-zid")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| AuthServerError::bad_request("Missing 0x-zid header"))?;

        // 1. Run the pre-request subroutines
        let mut ctx: RequestContext<ExternalMatchRequest> =
            self.preprocess_rfqt_quote_request(path, headers, body.clone(), query_str).await?;
        self.direct_match_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx): (
            BytesResponse,
            ResponseContext<ExternalMatchRequest, ExternalMatchResponse>,
        ) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.direct_match_post_request(raw_resp, ctx)?;

        // 4. Transform external match response to RFQT response
        let external_match_body = res.body();
        let external_match_resp: ExternalMatchResponse =
            serde_json::from_slice(external_match_body).map_err(AuthServerError::serde)?;
        let rfqt_request: RfqtQuoteRequest = json_deserialize(&body, true /* stringify */)?;
        let rfqt_response =
            transform_external_match_to_rfqt_response(&external_match_resp, rfqt_request)?;

        Ok(warp::reply::json(&rfqt_response))
    }

    /// Build request context for an external match related request, before the
    /// request is proxied to the relayer
    ///
    /// This method handles the specific case where we need to:
    /// 1. Authorize using the original RFQT request body (for HMAC validation)
    /// 2. Construct the RequestContext with the transformed external match
    ///    request body
    #[instrument(skip_all)]
    pub async fn preprocess_rfqt_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<RequestContext<ExternalMatchRequest>, AuthServerError> {
        // Authorize using the original RFQT body (for proper HMAC validation)
        let path = path.as_str().to_string();
        let (key_desc, key_id) = self.authorize_request(&path, &query_str, &headers, &body).await?;
        let sdk_version = get_sdk_version(&headers);

        // Parse the original RFQT request body
        let rfq_request = json_deserialize(&body, true /* stringify */)?;

        // Transform RFQT request to external match request
        let external_match_request = transform_rfqt_to_external_match_request(rfq_request)?;
        self.validate_request_body(&external_match_request)?;

        // Build the request context with the transformed external match request
        let mut ctx = RequestContext {
            path: REQUEST_EXTERNAL_MATCH_ROUTE.to_string(),
            query_str,
            sdk_version,
            headers,
            user: key_desc,
            key_id,
            body: external_match_request,
            sponsorship_info: None,
            request_id: Uuid::new_v4(),
        };

        // Set the relayer fee
        self.set_relayer_fee(&mut ctx).await?;
        Ok(ctx)
    }
}
