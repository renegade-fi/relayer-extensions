//! Handlers for external match endpoints

mod assemble_malleable_quote;
mod assemble_quote;
mod direct_match;
mod quote;

use auth_server_api::GasSponsorshipInfo;
use bytes::Bytes;
use http::{HeaderMap, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::AuthServerError, server::Server, telemetry::helpers::record_relayer_request_500,
    ApiError,
};
pub(crate) use assemble_malleable_quote::SponsoredAssembleMalleableQuoteResponseCtx;

use super::{get_sdk_version, log_unsuccessful_relayer_request};

// --------------------
// | Request Contexts |
// --------------------

// --- Request Context --- //

/// Context for an external match request
///
/// This struct handles the common logic for request context helpers used in
#[derive(Debug, Clone)]
pub struct RequestContext<Req: Serialize + for<'de> Deserialize<'de>> {
    /// The path of the request
    pub path: String,
    /// The query string of the request
    pub query_str: String,
    /// The API user description for the request
    ///
    /// Derived from the API key
    pub user: String,
    /// The version of the SDK used to make the request
    pub sdk_version: String,
    /// The headers of the request
    pub headers: HeaderMap,
    /// The body of the request
    pub body: Req,
    /// The gas sponsorship info for the request
    pub sponsorship_info: Option<GasSponsorshipInfo>,
    /// Unique ID for this request
    pub request_id: Uuid,
}

impl<Req: Serialize + for<'de> Deserialize<'de>> RequestContext<Req> {
    /// Get the user description for the request
    pub fn user(&self) -> String {
        self.user.to_string()
    }

    /// Get a reference to the query string
    pub fn query(&self) -> String {
        self.query_str.to_string()
    }

    /// Get a reference to the path
    pub fn path(&self) -> String {
        self.path.to_string()
    }

    /// Get a mutable reference to the request body
    pub fn body_mut(&mut self) -> &mut Req {
        &mut self.body
    }

    /// Get the json encoded body bytes
    pub fn body_bytes(&self) -> Result<Vec<u8>, AuthServerError> {
        serde_json::to_vec(&self.body).map_err(AuthServerError::serde)
    }

    /// Attach gas sponsorship info to the request context
    pub fn set_sponsorship_info(&mut self, info: Option<GasSponsorshipInfo>) {
        self.sponsorship_info = info;
    }
}

// --- Response Context --- //

/// Context for an external match response
#[derive(Debug, Clone)]
pub struct ResponseContext<
    Req: Serialize + for<'de> Deserialize<'de>,
    Resp: Serialize + for<'de> Deserialize<'de>,
> {
    /// The path of the request
    pub path: String,
    /// The query string of the request
    pub query_str: String,
    /// Derived from the API key
    pub user: String,
    /// The version of the SDK used to make the request
    pub sdk_version: String,
    /// The headers of the request
    pub headers: HeaderMap,
    /// A copy of the request
    pub request: Req,
    /// A copy of the raw response
    pub status: StatusCode,
    /// The deserialized response body
    ///
    /// May be `None` if the relayer returned a non-200 status code
    pub response: Option<Resp>,
    /// The gas sponsorship info for the request
    pub sponsorship_info: Option<GasSponsorshipInfo>,
    /// Unique ID for this request-response flow
    pub request_id: Uuid,
}

impl<
        Req: Serialize + for<'de> Deserialize<'de>,
        Resp: Serialize + for<'de> Deserialize<'de> + Clone,
    > ResponseContext<Req, Resp>
{
    /// Create a response context from a response and request context
    pub fn from_response(
        status: StatusCode,
        body: Option<Resp>,
        request: RequestContext<Req>,
    ) -> Result<ResponseContext<Req, Resp>, AuthServerError> {
        Ok(ResponseContext {
            path: request.path,
            query_str: request.query_str,
            user: request.user,
            sdk_version: request.sdk_version,
            headers: request.headers,
            request: request.body,
            status,
            response: body,
            sponsorship_info: request.sponsorship_info,
            request_id: request.request_id,
        })
    }

    /// Whether the relayer returned a 200 status code
    pub fn is_success(&self) -> bool {
        self.status == StatusCode::OK
    }

    /// Get the status code of the response
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Get a reference to the user description
    pub fn user(&self) -> String {
        self.user.to_string()
    }

    /// Get a reference to the request
    pub fn request(&self) -> &Req {
        &self.request
    }

    /// Get a reference to the response body unwrapping its optional
    ///
    /// Panics if the response body is `None`
    pub fn response(&self) -> Resp {
        self.response.as_ref().unwrap().clone()
    }

    /// Get a reference to the gas sponsorship info
    pub fn sponsorship_info(&self) -> Option<GasSponsorshipInfo> {
        self.sponsorship_info.clone()
    }
}

// ---------------------------
// | Request Context Helpers |
// ---------------------------

impl Server {
    /// Build request context for an external match related request, before the
    /// request is proxied to the relayer
    pub async fn preprocess_request<Req>(
        &self,
        path: warp::path::FullPath,
        headers: warp::hyper::HeaderMap,
        body: Bytes,
        query_str: String,
    ) -> Result<RequestContext<Req>, ApiError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
    {
        // Authorize the request
        let path = path.as_str().to_string();
        let key_desc = self.authorize_request(&path, &query_str, &headers, &body).await?;
        let sdk_version = get_sdk_version(&headers);

        // Deserialize the request body, then build the context
        let body: Req = serde_json::from_slice(&body).map_err(AuthServerError::serde)?;
        Ok(RequestContext {
            path,
            query_str,
            sdk_version,
            headers,
            user: key_desc,
            body,
            sponsorship_info: None,
            request_id: Uuid::new_v4(),
        })
    }

    /// Forward a request context to the relayer's admin API, returning the
    /// associated response context
    ///
    /// Returns the raw response from the relayer as well as the response
    /// context generated from the relayer's response
    pub async fn forward_request<Req, Resp>(
        &self,
        ctx: RequestContext<Req>,
    ) -> Result<(Response<Bytes>, ResponseContext<Req, Resp>), AuthServerError>
    where
        Req: Serialize + for<'de> Deserialize<'de>,
        Resp: Serialize + for<'de> Deserialize<'de> + Clone,
    {
        let body_bytes = ctx.body_bytes()?;
        let resp = self
            .send_admin_request(Method::POST, &ctx.path, ctx.headers.clone(), body_bytes.into())
            .await?;

        // Handle the status codes
        let status = resp.status();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            record_relayer_request_500(ctx.user(), ctx.path());
        } else if status != StatusCode::OK {
            log_unsuccessful_relayer_request(&resp, &ctx.user(), &ctx.path(), &ctx.headers);
        }

        // Deserialize the response body
        let response = if status == StatusCode::OK {
            let body = serde_json::from_slice(resp.body()).map_err(AuthServerError::serde)?;
            Some(body)
        } else {
            None
        };

        let ctx = ResponseContext::from_response(status, response, ctx)?;
        Ok((resp, ctx))
    }
}
