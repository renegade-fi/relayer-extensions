//! RFQT Quote endpoint handler

use bytes::Bytes;
use http::HeaderMap;
use tracing::instrument;
use uuid::Uuid;
use warp::reject::Rejection;

use renegade_api::http::external_match::{
    ExternalMatchRequest, ExternalMatchResponse, REQUEST_EXTERNAL_MATCH_ROUTE,
};

use auth_server_api::rfqt::RfqtQuoteRequest;

use crate::error::AuthServerError;
use crate::http_utils::request_response::overwrite_response_body;
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
    ) -> Result<BytesResponse, Rejection> {
        // 1. Run the pre-request subroutines
        let (mut ctx, rfqt_request) =
            self.rfqt_pre_request(path, headers, body.clone(), query_str).await?;
        self.direct_match_pre_request(&mut ctx).await?;

        // 2. Proxy the request to the relayer
        let (raw_resp, ctx) = self.forward_request(ctx).await?;

        // 3. Run the post-request subroutines
        let res = self.direct_match_post_request(raw_resp, ctx.clone())?;

        // 4. Run RFQT-specific post-processing
        let res = self.rfqt_post_request(res, &ctx, &rfqt_request)?;

        Ok(res)
    }

    /// Build request context for an external match related request, before the
    /// request is proxied to the relayer
    ///
    /// This method handles the specific case where we need to:
    /// 1. Authorize using the original RFQT request body (for HMAC validation)
    /// 2. Construct the RequestContext with the transformed external match
    ///    request body
    #[instrument(skip_all)]
    pub async fn rfqt_pre_request(
        &self,
        path: warp::path::FullPath,
        headers: HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<(RequestContext<ExternalMatchRequest>, RfqtQuoteRequest), AuthServerError> {
        // Authorize using the original RFQT body (for proper HMAC validation)
        let path = path.as_str().to_string();
        let (key_desc, key_id) = self.authorize_request(&path, &query_str, &headers, &body).await?;
        let sdk_version = get_sdk_version(&headers);

        // Parse the original RFQT request body
        let rfq_request: RfqtQuoteRequest = json_deserialize(&body, false /* stringify */)?;

        // Transform RFQT request to external match request
        let external_match_request = transform_rfqt_to_external_match_request(rfq_request.clone())?;
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
        Ok((ctx, rfq_request))
    }

    /// Run the post-request subroutines for the RFQT quote endpoint
    #[instrument(skip_all, fields(success = ctx.is_success(), status = ctx.status().as_u16()))]
    pub(crate) fn rfqt_post_request(
        &self,
        mut resp: BytesResponse,
        ctx: &ResponseContext<ExternalMatchRequest, ExternalMatchResponse>,
        rfqt_request: &RfqtQuoteRequest,
    ) -> Result<BytesResponse, AuthServerError> {
        // If the relayer returned non-200, return the response directly (e.g., 204 No
        // Content)
        if !ctx.is_success() {
            return Ok(resp);
        }

        // Deserialize the external match response
        let external_match_body = resp.body();
        let external_match_resp: ExternalMatchResponse =
            serde_json::from_slice(external_match_body).map_err(AuthServerError::serde)?;

        // Transform external match response to RFQT response
        let rfqt_response =
            transform_external_match_to_rfqt_response(&external_match_resp, rfqt_request.clone())?;

        // We don't stringify here to rely on the serialization defined
        // `RfqtQuoteResponse`
        overwrite_response_body(&mut resp, rfqt_response, false /* stringify */)?;

        Ok(resp)
    }
}
