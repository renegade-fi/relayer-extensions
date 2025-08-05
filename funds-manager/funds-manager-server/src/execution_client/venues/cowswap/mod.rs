//! Cowswap-specific logic for getting quotes and executing swaps.

use alloy::signers::local::PrivateKeySigner;
use alloy_primitives::U256;
use async_trait::async_trait;
use funds_manager_api::quoters::QuoteParams;
use renegade_common::types::chain::Chain;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{
    execution_client::{
        error::ExecutionClientError,
        venues::{
            cowswap::api_types::{OrderKind, OrderQuoteRequest, OrderQuoteResponse, SigningScheme},
            quote::{ExecutableQuote, ExecutionQuote, QuoteExecutionData},
            ExecutionResult, ExecutionVenue, SupportedExecutionVenue,
        },
    },
    helpers::handle_http_response,
};

pub mod api_types;

// -------------
// | Constants |
// -------------

/// The base URL for the Cowswap API
const COWSWAP_BASE_URL: &str = "https://api.cow.fi";

/// The path fragment containing the API version for Cowswap endpoints
const COWSWAP_API_VERSION_PATH_SEGMENT: &str = "api/v1";

/// The endpoint for requesting a Cowswap quote
const COWSWAP_QUOTE_ENDPOINT: &str = "quote";

// ---------
// | Types |
// ---------

/// The auxiliary data needed to execute a quote on Cowswap
#[derive(Debug, Clone)]
pub struct CowswapQuoteExecutionData {
    /// The Unix timestamp until which the order is valid
    pub valid_to: u32,
    /// Amount of sell token (in atoms) used to cover network fees.
    ///
    /// Needs to be zero (and incorporated into the limit price) when placing
    /// the order.
    pub fee_amount: U256,
    /// The kind of quote requested.
    pub kind: OrderKind,
    /// Whether the order is partially fillable (otherwise, fill-or-kill)
    pub partially_fillable: bool,
    /// The signature of the order
    pub signing_scheme: SigningScheme,
    /// The EIP-712 signature over the order.
    ///
    /// Concretely, the hex-encoded `r || s || v` values, totaling 65 bytes.
    // TODO: Determine if `v` is expected to be 0/1 or 27/28.
    pub signature: String,
    /// A string encoding of the JSON `app_data` that was used to request the
    /// quote.
    ///
    /// The UTF-8 encoding of this string must be the preimage of the `app_data`
    /// hash in the quote response.
    ///
    /// In our case, this should always be "{}".
    pub app_data: String,
}

impl ExecutableQuote {
    /// Convert a Cowswap quote into an executable quote
    pub fn from_cowswap_quote(
        cowswap_quote: OrderQuoteResponse,
        chain: Chain,
        private_key: &PrivateKeySigner,
    ) -> Self {
        let sell_token = cowswap_quote.get_sell_token(chain);
        let buy_token = cowswap_quote.get_buy_token(chain);
        let (sell_amount, buy_amount) = cowswap_quote.get_quote_amounts_after_costs();

        let quote = ExecutionQuote {
            sell_token,
            buy_token,
            sell_amount,
            buy_amount,
            venue: SupportedExecutionVenue::Cowswap,
            chain,
        };

        // When submitting an order, the `fee_amount` field is expected to be
        // set to zero, with the actual fee amount being folded into the
        // buy/sell amounts.
        // See the `OrderParameters` docs in https://docs.cow.fi/cow-protocol/reference/apis/orderbook
        // for more details.
        let fee_amount = U256::ZERO;

        let valid_to = cowswap_quote.compute_valid_to();
        let kind = cowswap_quote.get_order_kind();
        let partially_fillable = cowswap_quote.is_partially_fillable();
        let signing_scheme = cowswap_quote.get_signing_scheme();
        let app_data = cowswap_quote.get_app_data();
        let signature = cowswap_quote.sign_order(private_key);

        let execution_data = CowswapQuoteExecutionData {
            valid_to,
            fee_amount,
            kind,
            partially_fillable,
            signing_scheme,
            app_data,
            signature,
        };

        ExecutableQuote { quote, execution_data: QuoteExecutionData::Cowswap(execution_data) }
    }
}

// ----------
// | Client |
// ----------

/// A client for interacting with the Cowswap API
#[derive(Clone)]
pub struct CowswapClient {
    /// The underlying HTTP client
    http_client: Client,
    /// The hot wallet used for executing quotes
    hot_wallet: PrivateKeySigner,
    /// The chain on which the client is operating
    chain: Chain,
}

impl CowswapClient {
    /// Create a new client
    pub fn new(hot_wallet: PrivateKeySigner, chain: Chain) -> Self {
        Self { http_client: Client::new(), hot_wallet, chain }
    }

    /// Send a POST request to the Cowswap API
    async fn send_post_request<Req: Serialize, Res: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: Req,
    ) -> Result<Res, ExecutionClientError> {
        let url = self.build_cowswap_url(path)?;
        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Send a GET request to the Cowswap API
    async fn send_get_request<Res: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<Res, ExecutionClientError> {
        let url = self.build_cowswap_url(path)?;
        let response = self.http_client.get(url).send().await?;

        handle_http_response(response).await.map_err(ExecutionClientError::http)
    }

    /// Build a Cowswap API URL for a given path
    fn build_cowswap_url(&self, path: &str) -> Result<String, ExecutionClientError> {
        let cowswap_chain = to_cowswap_chain(self.chain)?;
        Ok(format!("{COWSWAP_BASE_URL}/{cowswap_chain}/{COWSWAP_API_VERSION_PATH_SEGMENT}/{path}"))
    }

    /// Construct a Cowswap quote request from a QuoteParams struct
    fn construct_quote_params(&self, params: QuoteParams) -> OrderQuoteRequest {
        OrderQuoteRequest {
            sell_token: params.from_token,
            buy_token: params.to_token,
            from: self.hot_wallet.address().to_string(),
            kind: OrderKind::Sell,
            sell_amount_before_fee: params.from_amount,
        }
    }
}

// ------------------------
// | Execution Venue Impl |
// ------------------------

#[async_trait]
impl ExecutionVenue for CowswapClient {
    /// Get a quote from the Cowswap API
    async fn get_quote(
        &self,
        params: QuoteParams,
    ) -> Result<ExecutableQuote, ExecutionClientError> {
        let quote_request = self.construct_quote_params(params);
        let quote_response: OrderQuoteResponse =
            self.send_post_request(COWSWAP_QUOTE_ENDPOINT, quote_request).await?;

        let executable_quote =
            ExecutableQuote::from_cowswap_quote(quote_response, self.chain, &self.hot_wallet);

        Ok(executable_quote)
    }

    /// Execute a quote from the Cowswap API
    async fn execute_quote(
        &self,
        _executable_quote: &ExecutableQuote,
    ) -> Result<ExecutionResult, ExecutionClientError> {
        todo!()
    }
}

// -----------
// | Helpers |
// -----------

/// Convert a `Chain` to a Cowswap chain name
fn to_cowswap_chain(chain: Chain) -> Result<String, ExecutionClientError> {
    match chain {
        Chain::ArbitrumOne => Ok("arbitrum_one".to_string()),
        Chain::BaseMainnet => Ok("base".to_string()),
        _ => Err(ExecutionClientError::onchain(format!("Cowswap does not support chain: {chain}"))),
    }
}
