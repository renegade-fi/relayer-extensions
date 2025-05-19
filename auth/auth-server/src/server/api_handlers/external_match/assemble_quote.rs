//! Assemble quote endpoint handler

use bytes::Bytes;
use http::{Method, StatusCode};
use renegade_api::http::external_match::AssembleExternalMatchRequest;
use tracing::{instrument, warn};
use warp::{reject::Rejection, reply::Reply};

use crate::{
    error::AuthServerError,
    http_utils::overwrite_response_body,
    server::{api_handlers::log_unsuccessful_relayer_request, Server},
    telemetry::helpers::record_relayer_request_500,
};

impl Server {
    /// Handle an external quote-assembly request
    #[instrument(skip(self, path, headers, body))]
    pub async fn handle_assemble_quote_request(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<impl Reply, Rejection> {
        // Authorize the request
        let path_str = path.as_str();
        let key_desc = self.authorize_request(path_str, &query_str, &headers, &body).await?;

        // Check the bundle rate limit
        let mut req: AssembleExternalMatchRequest =
            serde_json::from_slice(&body).map_err(AuthServerError::serde)?;
        self.check_bundle_rate_limit(key_desc.clone(), req.allow_shared).await?;

        // Update the request to remove the effects of gas sponsorship, if
        // necessary
        let gas_sponsorship_info =
            self.maybe_update_assembly_request_with_gas_sponsorship(&mut req).await?;

        // Serialize the potentially updated request body
        let req_body = serde_json::to_vec(&req).map_err(AuthServerError::serde)?;

        // Send the request to the relayer
        let mut res = self
            .send_admin_request(Method::POST, path_str, headers.clone(), req_body.clone().into())
            .await?;

        let status = res.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(key_desc.clone(), path_str.to_string());
        }
        if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&res, &key_desc, path_str, &req_body, &headers);
            return Ok(res);
        }

        // Apply gas sponsorship to the resulting bundle, if necessary
        let sponsored_match_resp =
            self.maybe_apply_gas_sponsorship_to_match_response(res.body(), gas_sponsorship_info)?;
        overwrite_response_body(&mut res, sponsored_match_resp.clone())?;

        // Record the bundle context in the store
        let bundle_id = self
            .write_bundle_context(
                &sponsored_match_resp,
                &headers,
                key_desc.clone(),
                req.allow_shared,
            )
            .await?;

        let server_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = server_clone.handle_quote_assembly_bundle_response(
                &key_desc,
                &req,
                &headers,
                &sponsored_match_resp,
                &bundle_id,
            ) {
                warn!("Error handling bundle: {e}");
            };
        });

        Ok(res)
    }
}
