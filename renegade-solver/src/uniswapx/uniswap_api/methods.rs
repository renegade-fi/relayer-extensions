//! Code for interacting with the UniswapX API

use tracing::error;
use url::form_urlencoded;

use crate::{
    error::SolverResult,
    uniswapx::{
        uniswap_api::types::{GetOrdersResponse, OrderEntity},
        UniswapXSolver,
    },
};

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

impl UniswapXSolver {
    /// Fetch open orders from the UniswapX API
    pub(crate) async fn fetch_open_orders(&self) -> SolverResult<Vec<OrderEntity>> {
        let url = self.build_request_url();
        let response = self.http_client.get(&url).send().await?;

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

        let mut orders = Vec::new();
        for order in orders_response.orders {
            if !self.is_order_processed(&order).await {
                orders.push(order);
            }
        }

        Ok(orders)
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
