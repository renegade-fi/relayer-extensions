//! API types for the auth server

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_ref_mut)]
#![feature(trivial_bounds)]

use alloy_primitives::{ruint::FromUintError, Address, U256};
use renegade_api::http::external_match::{
    AssembleExternalMatchRequest, AtomicMatchApiBundle, ExternalOrder, SignedExternalQuote,
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
/// gas sponsorship info.
///
/// We manually flatten the fields of
/// [`renegade_api::http::external_match::ExternalQuoteResponse`]
/// into this struct, as `serde` does not support `u128`s when using
/// `#[serde(flatten)]`:
/// https://github.com/serde-rs/json/issues/625
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SponsoredQuoteResponse {
    /// The external quote response from the relayer, potentially updated to
    /// reflect the post-sponsorship price and receive amount
    pub signed_quote: SignedExternalQuote,
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

impl GasSponsorshipInfo {
    /// Construct a new gas sponsorship info struct from strongly-typed
    /// parameters
    pub fn new(
        refund_amount: U256,
        refund_native_eth: bool,
        refund_address: Address,
    ) -> Result<Self, String> {
        let refund_amount: u128 =
            refund_amount.try_into().map_err(|e: FromUintError<u128>| e.to_string())?;

        let refund_address =
            if refund_address.is_zero() { None } else { Some(format!("{:#x}", refund_address)) };

        Ok(Self { refund_amount, refund_native_eth, refund_address })
    }

    /// Whether this sponsorship implies an update to the effective price /
    /// receive amount of the associated match result
    pub fn requires_match_result_update(&self) -> bool {
        !self.refund_native_eth && self.refund_address.is_none()
    }

    /// Get the refund amount as an alloy U256
    pub fn get_refund_amount(&self) -> U256 {
        U256::from(self.refund_amount)
    }

    /// Get the refund address as an alloy address, defaulting to the zero
    /// address if not provided or malformed
    pub fn get_refund_address(&self) -> Address {
        self.refund_address
            .as_ref()
            .map(|s| s.parse().unwrap_or(Address::ZERO))
            .unwrap_or(Address::ZERO)
    }
}

/// A request to assemble a potentially sponsored quote into a settlement bundle
///
/// We manually flatten the fields of [`AssembleExternalMatchRequest`]
/// into this struct, as `serde` does not support `u128`s when using
/// `#[serde(flatten)]`:
/// https://github.com/serde-rs/json/issues/625
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssembleSponsoredMatchRequest {
    /// Whether or not to include gas estimation in the response
    #[serde(default)]
    pub do_gas_estimation: bool,
    /// The receiver address of the match, if not the message sender
    #[serde(default)]
    pub receiver_address: Option<String>,
    /// The updated order if any changes have been made
    #[serde(default)]
    pub updated_order: Option<ExternalOrder>,
    /// The signed quote
    pub signed_quote: SignedExternalQuote,
    /// The signed gas sponsorship info associated with the quote,
    /// if sponsorship was requested
    pub gas_sponsorship_info: Option<SignedGasSponsorshipInfo>,
}

impl AssembleSponsoredMatchRequest {
    /// Extract an [`AssembleExternalMatchRequest`]
    pub fn assemble_external_match_request(&self) -> AssembleExternalMatchRequest {
        AssembleExternalMatchRequest {
            do_gas_estimation: self.do_gas_estimation,
            receiver_address: self.receiver_address.clone(),
            updated_order: self.updated_order.clone(),
            signed_quote: self.signed_quote.clone(),
        }
    }
}

/// A sponsored match response from the auth server
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SponsoredMatchResponse {
    /// The external match bundle, potentially updated to reflect the
    /// post-sponsorship receive amount
    pub match_bundle: AtomicMatchApiBundle,
    /// Whether or not the match was sponsored
    pub is_sponsored: bool,
    /// The gas sponsorship info
    pub gas_sponsorship_info: Option<GasSponsorshipInfo>,
}

/// The query parameters used for gas sponsorship
#[derive(Debug, Serialize, Deserialize)]
pub struct GasSponsorshipQueryParams {
    /// Whether to use gas sponsorship for the external match.
    #[deprecated(since = "0.1.0", note = "Use `disable_gas_sponsorship` instead")]
    pub use_gas_sponsorship: Option<bool>,
    /// Whether to disable gas sponsorship for the external match.
    pub disable_gas_sponsorship: Option<bool>,
    /// The address to refund gas to.
    /// In the case of a native ETH refund, defaults to `tx::origin`.
    /// In the case of an in-kind refund, defaults to the receiver.
    pub refund_address: Option<String>,
    /// Whether to provide the gas refund in terms of native ETH,
    /// as opposed to the buy-side token.
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

    /// Get the gas sponsorship parameters, defaulting to in-kind gas
    /// sponsorship
    pub fn get_or_default(&self) -> (bool, Address, bool) {
        (
            self.disable_gas_sponsorship.unwrap_or(false),
            self.get_refund_address(),
            self.refund_native_eth.unwrap_or(false),
        )
    }
}
