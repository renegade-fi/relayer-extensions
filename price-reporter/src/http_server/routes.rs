//! The routes for the HTTP server

use async_trait::async_trait;
use hyper::{Body, Request, Response, StatusCode};
use renegade_common::types::Price;
use renegade_price_reporter::worker::ExchangeConnectionsConfig;

use crate::{
    errors::ServerError,
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
