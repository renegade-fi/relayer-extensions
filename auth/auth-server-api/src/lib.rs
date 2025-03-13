//! API types for the auth server

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

use alloy_primitives::Address;
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, AtomicMatchApiBundle, ExternalQuoteResponse,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The Renegade API key header
pub const RENEGADE_API_KEY_HEADER: &str = "X-Renegade-Api-Key";

// ----------------------
// | API Key Management |
// ----------------------

/// The path to create a new API key
///
/// POST /api-keys
pub const API_KEYS_PATH: &str = "api-keys";
/// The path to mark an API key as inactive
///
/// POST /api-keys/{id}/deactivate
pub const DEACTIVATE_API_KEY_PATH: &str = "/api-keys/{id}/deactivate";

/// A request to create a new API key
#[derive(Debug, Serialize, Deserialize)]
pub struct CreateApiKeyRequest {
    /// The API key id
    pub id: Uuid,
    /// The API key secret
    ///
    /// Expected as a base64 encoded string
    pub secret: String,
    /// A description of the API key's purpose
    pub description: String,
}

/// An external quote response from the auth server, potentially with
/// gas sponsorship info
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SponsoredQuoteResponse {
    /// The external quote response from the relayer, potentially updated to
    /// reflect the post-sponsorship price and receive amount
    #[serde(flatten)]
    pub external_quote_response: ExternalQuoteResponse,
    /// The signed gas sponsorship info, if sponsorship was requested
    pub gas_sponsorship_info: Option<SignedGasSponsorshipInfo>,
}

/// Signed metadata regarding gas sponsorship for a quote
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedGasSponsorshipInfo {
    /// The signed gas sponsorship info
    pub gas_sponsorship_info: GasSponsorshipInfo,
    /// The auth server's signature over the gas sponsorship info
    pub signature: String,
}

/// Metadata regarding gas sponsorship for a quote
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GasSponsorshipInfo {
    /// The amount to be refunded as a result of gas sponsorship.
    /// This amount is firm, it will not change when the quote is assembled.
    pub refund_amount: u128,
    /// Whether the refund is in terms of native ETH.
    pub refund_native_eth: bool,
    /// The address to which the refund will be sent, if set explicitly.
    pub refund_address: Option<String>,
}

/// A request to assemble a potentially sponsored quote into a settlement bundle
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssembleSponsoredMatchRequest {
    /// The request to assemble the external match
    #[serde(flatten)]
    pub assemble_external_match_request: AssembleExternalMatchRequest,
    /// The gas sponsorship info associated with the quote,
    /// if sponsorship was requested
    pub gas_sponsorship_info: Option<SignedGasSponsorshipInfo>,
}

/// A sponsored match response from the auth server
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SponsoredMatchResponse {
    /// The external match bundle
    pub match_bundle: AtomicMatchApiBundle,
    /// Whether or not the match was sponsored
    pub is_sponsored: bool,
}

/// The query parameters used for gas sponsorship
#[derive(Debug, Serialize, Deserialize)]
pub struct GasSponsorshipQueryParams {
    /// Whether to use gas sponsorship for the external match.
    /// Defaults to `true`.
    pub use_gas_sponsorship: Option<bool>,
    /// The address to refund gas to.
    /// In the case of a native ETH refund, defaults to `tx::origin`.
    /// In the case of an in-kind refund, defaults to the receiver.
    pub refund_address: Option<String>,
    /// Whether to provide the gas refund in terms of native ETH,
    /// as opposed to the buy-side token.
    /// Defaults to `false`, meaning the buy-side token is used.
    pub refund_native_eth: Option<bool>,
}

impl GasSponsorshipQueryParams {
    /// Get the refund address, defaulting to the zero address if not provided
    /// or malformed
    pub fn get_refund_address(&self) -> Address {
        self.refund_address
            .as_ref()
            .map(|s| s.parse().unwrap_or(Address::ZERO))
            .unwrap_or(Address::ZERO)
    }

    /// Get the gas sponsorship parameters, defaulting to the
    /// server's defaults if not provided
    pub fn get_or_default(&self) -> (bool, Address, bool) {
        (
            self.use_gas_sponsorship.unwrap_or(true),
            self.get_refund_address(),
            self.refund_native_eth.unwrap_or(false),
        )
    }
}
