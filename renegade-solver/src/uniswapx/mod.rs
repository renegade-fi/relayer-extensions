//! UniswapX API client and handlers

use std::time::Duration;

use reqwest::Client as ReqwestClient;
use tracing::{error, info};
use url::form_urlencoded;

use crate::{error::SolverResult, uniswapx::api_types::OrderEntity};

mod api_types;
use api_types::GetOrdersResponse;

/// The interval at which to poll for new orders
const POLLING_INTERVAL: Duration = Duration::from_secs(1);

/// The order status parameter
const ORDER_STATUS_PARAM: &str = "orderStatus";
/// The chain ID parameter
const CHAIN_ID_PARAM: &str = "chainId";
/// The order type parameter
const ORDER_TYPE_PARAM: &str = "orderType";

/// Query parameter values
/// The order status value
const ORDER_STATUS_OPEN: &str = "open";
/// The chain ID for base
const CHAIN_ID_BASE: &str = "8453";
/// The order type for priority orders
const ORDER_TYPE_PRIORITY: &str = "Priority";

/// The UniswapX API client
#[derive(Clone)]
pub struct UniswapXSolver {
    /// The base URL for the UniswapX API
    base_url: String,
    /// The API client
    client: ReqwestClient,
}

impl UniswapXSolver {
    /// Create a new UniswapX solver
    pub fn new(base_url: String) -> Self {
        Self { base_url, client: ReqwestClient::new() }
    }

    /// Spawn a polling loop for the UniswapX API
    pub fn spawn_polling_loop(&self) {
        let self_clone = self.clone();
        tokio::spawn(async move {
            if let Err(e) = self_clone.polling_loop().await {
                error!("Polling loop error: {e}");
                error!("Critical error in polling loop, shutting down process");
                std::process::exit(1);
            }
        });
    }

    /// The inner polling loop
    async fn polling_loop(&self) -> SolverResult<()> {
        loop {
            tokio::time::sleep(POLLING_INTERVAL).await;
            if let Err(e) = self.poll_orders().await {
                error!("Error polling for orders: {e}");
                continue;
            }
        }
    }

    /// Poll the UniswapX API for new orders
    async fn poll_orders(&self) -> SolverResult<()> {
        // For now, just print each order
        let orders = self.fetch_open_orders().await?;
        for order in &orders {
            let input = &order.input;
            let first_output = &order.outputs[0];
            info!(
                "Found order for {} {} -> {} {}",
                input.amount, input.token, first_output.amount, first_output.token
            );
        }

        // TODO: Process each order
        Ok(())
    }

    /// Fetch open orders from the UniswapX API
    async fn fetch_open_orders(&self) -> SolverResult<Vec<OrderEntity>> {
        let url = self.build_request_url();
        let response = self.client.get(&url).send().await?;

        // Check if the response is successful
        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("API request failed with status: {status}: {error_text}");
            return Ok(vec![]);
        }

        // Deserialize the JSON response
        let response_text = response.text().await?;
        let orders_response: GetOrdersResponse = serde_json::from_str(&response_text)?;
        Ok(orders_response.orders)
    }

    /// Build the request URL for the UniswapX API
    ///
    /// See docs [here](https://docs.uniswap.org/contracts/uniswapx/guides/priority/priorityorderreactor#retrieving-and-executing-signed-orders)
    fn build_request_url(&self) -> String {
        let query_params = Self::get_default_query_params();
        let mut query_string = form_urlencoded::Serializer::new(String::new());
        for (key, value) in query_params {
            query_string.append_pair(key, value);
        }
        let query = query_string.finish();
        format!("{}/orders?{}", self.base_url, query)
    }

    /// Get the default query parameters for fetching orders
    fn get_default_query_params() -> Vec<(&'static str, &'static str)> {
        vec![
            (ORDER_STATUS_PARAM, ORDER_STATUS_OPEN),
            (CHAIN_ID_PARAM, CHAIN_ID_BASE),
            (ORDER_TYPE_PARAM, ORDER_TYPE_PRIORITY),
        ]
    }
}
