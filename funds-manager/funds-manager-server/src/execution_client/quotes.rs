//! Client methods for fetching quotes and prices from the execution venue

use std::collections::HashMap;

use serde_json;

use super::{error::ExecutionClientError, ExecutionClient};

/// The quote endpoint
const QUOTE_ENDPOINT: &str = "v1/quote";

impl ExecutionClient {
    /// Fetch a quote by forwarding raw query parameters
    pub async fn get_quote(
        &self,
        query_params: HashMap<String, String>,
    ) -> Result<serde_json::Value, ExecutionClientError> {
        let params: Vec<(&str, &str)> =
            query_params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

        self.send_get_request(QUOTE_ENDPOINT, &params).await
    }
}
