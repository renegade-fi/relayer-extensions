//! Client methods for fetching quotes and prices from the execution venue

use funds_manager_api::quoters::LiFiQuoteParams;
use tracing::instrument;

use crate::execution_client::venues::{lifi::LifiQuote, quote::ExecutableQuote};

use super::{error::ExecutionClientError, ExecutionClient};

/// The quote endpoint
const QUOTE_ENDPOINT: &str = "v1/quote";

impl ExecutionClient {
    /// Fetch a quote by forwarding raw query parameters
    #[instrument(
        skip(self, params),
        fields(
            from_token = %params.from_token,
            to_token = %params.to_token,
            from_amount = %params.from_amount
        )
    )]
    pub async fn get_quote(
        &self,
        params: LiFiQuoteParams,
    ) -> Result<ExecutableQuote, ExecutionClientError> {
        let qs_config = serde_qs::Config::new().array_format(serde_qs::ArrayFormat::Unindexed);
        let query_string = qs_config.serialize_string(&params).unwrap();
        let url = format!("{}?{}", QUOTE_ENDPOINT, query_string);

        let resp: LifiQuote = self.send_get_request(&url).await?;

        ExecutableQuote::from_lifi_quote(resp, self.chain)
    }
}
