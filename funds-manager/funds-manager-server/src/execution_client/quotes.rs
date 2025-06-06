//! Client methods for fetching quotes and prices from the execution venue

use funds_manager_api::quoters::{ExecutionQuote, LiFiQuoteParams};
use funds_manager_api::venue::LiFiQuote;

use super::{error::ExecutionClientError, ExecutionClient};

/// The quote endpoint
const QUOTE_ENDPOINT: &str = "v1/quote";

impl ExecutionClient {
    /// Fetch a quote by forwarding raw query parameters
    pub async fn get_quote(
        &self,
        params: LiFiQuoteParams,
    ) -> Result<ExecutionQuote, ExecutionClientError> {
        let qs_config = serde_qs::Config::new().array_format(serde_qs::ArrayFormat::Unindexed);
        let query_string = qs_config.serialize_string(&params).unwrap();
        let url = format!("{}?{}", QUOTE_ENDPOINT, query_string);

        let resp: LiFiQuote = self.send_get_request(&url).await?;
        Ok(resp.into())
    }
}
