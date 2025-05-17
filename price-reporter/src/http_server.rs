//! The minimal HTTP server maintained by the price reporter

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use hyper::{
    server::conn::AddrStream,
    service::{make_service_fn, service_fn},
    Body, Error as HyperError, Request, Response, Server, StatusCode,
};
use matchit::Router;
use routes::{RefreshTokenMappingHandler, REFRESH_TOKEN_MAPPING_ROUTE};

use crate::{
    errors::ServerError,
    utils::{HttpRouter, PriceReporterConfig},
    ws_server::GlobalPriceStreams,
};

use self::routes::{Handler, HealthCheckHandler, PriceHandler, HEALTH_CHECK_ROUTE, PRICE_ROUTE};

pub mod routes;

/// The HTTP server for the price reporter
#[derive(Clone)]
pub struct HttpServer {
    /// The port on which the server will listen
    port: u16,
    /// The router for the HTTP server, used to match routes
    router: Arc<HttpRouter>,
}

impl HttpServer {
    /// Create a new HTTP server with the given port and global price streams
    pub fn new(config: &PriceReporterConfig, price_streams: GlobalPriceStreams) -> Self {
        let router = Self::build_router(config, price_streams);
        Self { port: config.http_port, router: Arc::new(router) }
    }

    /// Build the router for the HTTP server
    fn build_router(config: &PriceReporterConfig, price_streams: GlobalPriceStreams) -> HttpRouter {
        let mut router: Router<Box<dyn Handler>> = Router::new();

        router.insert(HEALTH_CHECK_ROUTE, Box::new(HealthCheckHandler::new())).unwrap();

        router
            .insert(
                PRICE_ROUTE,
                Box::new(PriceHandler::new(
                    config.exchange_conn_config.clone(),
                    price_streams.clone(),
                )),
            )
            .unwrap();

        router
            .insert(
                REFRESH_TOKEN_MAPPING_ROUTE,
                Box::new(RefreshTokenMappingHandler::new(
                    config.admin_key,
                    config.token_remap_path.clone(),
                    config.chains.clone(),
                    price_streams,
                    config.exchange_conn_config.clone(),
                    config.disabled_exchanges.clone(),
                )),
            )
            .unwrap();

        router
    }

    /// Serve an http request
    async fn serve_request(&self, req: Request<Body>) -> Response<Body> {
        if let Ok(matched_path) = self.router.at(req.uri().path()) {
            let handler = matched_path.value;
            let url_params =
                matched_path.params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            handler.as_ref().handle(req, url_params).await
        } else {
            Response::builder().status(StatusCode::NOT_FOUND).body(Body::from("Not Found")).unwrap()
        }
    }

    /// The execution loop for the http server, accepts incoming connections,
    /// serves them, and awaits the next connection
    pub async fn execution_loop(self) -> Result<(), ServerError> {
        // Build an HTTP handler callback
        // Clone self and move it into each layer of the callback so that each
        // scope has its own copy of self
        let self_clone = self.clone();
        let make_service = make_service_fn(move |_: &AddrStream| {
            let self_clone = self_clone.clone();
            async move {
                Ok::<_, Infallible>(service_fn(move |req: Request<Body>| {
                    let self_clone = self_clone.clone();
                    async move { Ok::<_, HyperError>(self_clone.serve_request(req).await) }
                }))
            }
        });

        // Build the http server and enter its execution loopx
        let addr: SocketAddr = format!("0.0.0.0:{}", self.port).parse().unwrap();
        Server::bind(&addr)
            .serve(make_service)
            .await
            .map_err(|err| ServerError::HttpServer(err.to_string()))
    }
}
