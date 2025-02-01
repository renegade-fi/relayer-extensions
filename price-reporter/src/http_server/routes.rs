//! The routes for the HTTP server

use async_trait::async_trait;
use hyper::{body::to_bytes, Body, Request, Response, StatusCode};
use renegade_api::auth::validate_expiring_auth;
use renegade_arbitrum_client::constants::Chain;
use renegade_common::types::{exchange::Exchange, wallet::keychain::HmacKey, Price};
use renegade_config::setup_token_remaps;
use renegade_price_reporter::worker::ExchangeConnectionsConfig;
use renegade_util::err_str;

use crate::{
    errors::ServerError,
    init_default_price_streams,
    utils::{parse_pair_info_from_topic, UrlParams},
    ws_server::GlobalPriceStreams,
};

/// A handler is attached to a route and handles the process of translating an
/// abstract request type into a response
#[async_trait]
pub trait Handler: Send + Sync {
    /// The handler method for the request/response on the handler's route
    async fn handle(&self, req: Request<Body>, url_params: UrlParams) -> Response<Body>;
}

// ----------------------
// | HEALTH CHECK ROUTE |
// ----------------------

/// The route for the health check endpoint
pub const HEALTH_CHECK_ROUTE: &str = "/health";

/// The handler for the health check endpoint
pub struct HealthCheckHandler;

impl HealthCheckHandler {
    /// Create a new health check handler
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Handler for HealthCheckHandler {
    async fn handle(&self, _: Request<Body>, _: UrlParams) -> Response<Body> {
        Response::builder().status(StatusCode::OK).body(Body::from("OK")).unwrap()
    }
}

// ---------------
// | PRICE ROUTE |
// ---------------

/// The route for the price endpoint
pub const PRICE_ROUTE: &str = "/price/:topic";

/// The handler for the price endpoint
#[derive(Clone)]
pub struct PriceHandler {
    /// The configuration for the exchange connections, used to potentially
    /// instantiate new price streams
    config: ExchangeConnectionsConfig,
    /// The global map of price streams, from which to read the price
    price_streams: GlobalPriceStreams,
}

impl PriceHandler {
    /// Create a new price handler with the given global price streams
    pub fn new(config: ExchangeConnectionsConfig, price_streams: GlobalPriceStreams) -> Self {
        Self { config, price_streams }
    }

    /// Get a single price from the stream pertaining to the given topic
    pub async fn get_price(&self, topic: &str) -> Result<Price, ServerError> {
        let self_clone = self.clone();

        let pair_info = parse_pair_info_from_topic(topic)?;
        let price_rx = self_clone
            .price_streams
            .get_or_create_price_stream(pair_info, self_clone.config.clone())
            .await?;

        let price = *price_rx.borrow();
        Ok(price)
    }
}

#[async_trait]
impl Handler for PriceHandler {
    async fn handle(&self, _: Request<Body>, url_params: UrlParams) -> Response<Body> {
        let topic = url_params.get("topic").unwrap();

        match self.get_price(topic).await {
            Ok(price) => Response::builder()
                .status(StatusCode::OK)
                .body(Body::from(price.to_string()))
                .unwrap(),
            Err(e) => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(e.to_string()))
                .unwrap(),
        }
    }
}

// -------------------------------
// | REFRESH TOKEN MAPPING ROUTE |
// -------------------------------

/// The route for the token mapping refresh endpoint
pub const REFRESH_TOKEN_MAPPING_ROUTE: &str = "/refresh-token-mapping";

/// The handler for the token mapping refresh endpoint
#[derive(Clone)]
pub struct RefreshTokenMappingHandler {
    /// The HMAC key for the admin API
    admin_key: Option<HmacKey>,
    /// The path to the token remap file
    token_remap_path: Option<String>,
    /// The chain to use for the token remap
    remap_chain: Chain,
    /// The global price streams
    price_streams: GlobalPriceStreams,
    /// The configuration for the exchange connections
    config: ExchangeConnectionsConfig,
    /// The exchanges for which to disable price reporting
    disabled_exchanges: Vec<Exchange>,
}

impl RefreshTokenMappingHandler {
    /// Create a new token mapping refresh handler
    pub fn new(
        admin_key: Option<HmacKey>,
        token_remap_path: Option<String>,
        remap_chain: Chain,
        price_streams: GlobalPriceStreams,
        config: ExchangeConnectionsConfig,
        disabled_exchanges: Vec<Exchange>,
    ) -> Self {
        Self { admin_key, token_remap_path, remap_chain, price_streams, config, disabled_exchanges }
    }

    /// Authenticate a token mapping refresh request using the admin HMAC key.
    async fn authenticate_request(&self, req: &mut Request<Body>) -> Result<(), ServerError> {
        if self.admin_key.is_none() {
            return Err(ServerError::NoAdminKey);
        }

        let req_body = to_bytes(req.body_mut()).await.map_err(err_str!(ServerError::Serde))?;
        let path = req.uri().path();
        let headers = req.headers();
        validate_expiring_auth(path, headers, &req_body, &self.admin_key.unwrap())
            .map_err(err_str!(ServerError::Unauthorized))
    }

    /// Refresh the token mapping from the remote source
    pub async fn refresh_token_mapping(&self) -> Result<(), ServerError> {
        let token_remap_path = self.token_remap_path.clone();
        let remap_chain = self.remap_chain;
        tokio::task::spawn_blocking(move || setup_token_remaps(token_remap_path, remap_chain))
            .await
            .map_err(err_str!(ServerError::TokenRemap))
            .and_then(|res| res.map_err(err_str!(ServerError::TokenRemap)))?;

        // Re-initialize the default price streams after refreshing the token mapping
        init_default_price_streams(
            &self.price_streams,
            &self.config,
            self.disabled_exchanges.clone(),
        )
    }
}

#[async_trait]
impl Handler for RefreshTokenMappingHandler {
    async fn handle(&self, mut req: Request<Body>, _: UrlParams) -> Response<Body> {
        if self.admin_key.is_none() {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Body::from("Admin API disabled".to_string()))
                .unwrap();
        }

        if let Err(e) = self.authenticate_request(&mut req).await {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::from(e.to_string()))
                .unwrap();
        }

        match self.refresh_token_mapping().await {
            Ok(_) => Response::builder().status(StatusCode::OK).body(Body::empty()).unwrap(),
            Err(e) => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from(e.to_string()))
                .unwrap(),
        }
    }
}
