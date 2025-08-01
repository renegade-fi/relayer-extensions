//! The minimal HTTP server maintained by the price reporter

use std::{net::SocketAddr, sync::Arc};

use http_body_util::Full;
use hyper::{
    body::{Bytes as BytesBody, Incoming as IncomingBody},
    server::conn::http1::Builder as Http1Builder,
    service::service_fn,
    Error as HyperError, Request, Response, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use matchit::Router;
use renegade_util::err_str;
use routes::{RefreshTokenMappingHandler, REFRESH_TOKEN_MAPPING_ROUTE};
use tokio::net::{TcpListener, TcpStream};
use tracing::error;

use crate::{
    errors::ServerError,
    price_stream_manager::GlobalPriceStreams,
    utils::{HttpRouter, PriceReporterConfig},
};

use self::routes::{Handler, HealthCheckHandler, PriceHandler, HEALTH_CHECK_ROUTE, PRICE_ROUTE};

pub mod routes;

/// A type for the full response body
pub(super) type ResponseBody = Full<BytesBody>;

/// Create a response body from an `Into` type
pub(crate) fn resp_body<T: Into<BytesBody>>(body: T) -> ResponseBody {
    Full::new(body.into())
}

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
    async fn serve_request(&self, req: Request<IncomingBody>) -> Response<ResponseBody> {
        if let Ok(matched_path) = self.router.at(req.uri().path()) {
            let handler = matched_path.value;
            let url_params =
                matched_path.params.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            handler.as_ref().handle(req, url_params).await
        } else {
            Response::builder().status(StatusCode::NOT_FOUND).body(resp_body("Not found")).unwrap()
        }
    }

    /// The execution loop for the http server, accepts incoming connections,
    /// serves them, and awaits the next connection
    pub async fn execution_loop(self) -> Result<(), ServerError> {
        // Await incoming connections and spawn a new task to handle each one
        let addr: SocketAddr = format!("0.0.0.0:{}", self.port).parse().unwrap();
        let listener = TcpListener::bind(addr).await.map_err(err_str!(ServerError::HttpServer))?;
        loop {
            let (stream, _) = listener.accept().await.map_err(err_str!(ServerError::HttpServer))?;
            let self_clone = self.clone();
            tokio::spawn(async move {
                if let Err(e) = self_clone.handle_stream(stream).await {
                    error!("Error handling stream: {e}");
                }
            });
        }
    }

    /// Handle an incoming TCP stream
    async fn handle_stream(&self, stream: TcpStream) -> Result<(), ServerError> {
        // Create the service function for the HTTP server
        let service_fn = service_fn(move |req: Request<IncomingBody>| {
            let self_clone = self.clone();
            async move { Ok::<_, HyperError>(self_clone.serve_request(req).await) }
        });

        let stream_io = TokioIo::new(stream);
        let timer = TokioTimer::new();
        Http1Builder::new()
            .timer(timer)
            .serve_connection(stream_io, service_fn)
            .await
            .map_err(err_str!(ServerError::HttpServer))
    }
}
