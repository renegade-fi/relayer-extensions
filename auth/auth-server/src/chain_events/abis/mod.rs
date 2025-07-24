//! ABI helpers for the chain events listener
use renegade_api::http::external_match::{ApiBoundedMatchResult, ApiExternalMatchResult};
use renegade_circuit_types::{
    Amount,
    r#match::{BoundedMatchResult, ExternalMatchResult},
    wallet::Nullifier,
};

use crate::{
    bundle_store::helpers::{generate_bundle_id, generate_malleable_bundle_id},
    error::AuthServerError,
};

#[cfg(feature = "arbitrum")]
mod arbitrum;
#[cfg(feature = "base")]
mod base;

#[cfg(feature = "arbitrum")]
pub(crate) use arbitrum::*;
#[cfg(feature = "base")]
pub(crate) use base::*;

/// An external match in the darkpool
pub enum ExternalMatch {
    /// A normal external match
    Standard(ExternalMatchResult),
    /// A malleable external match with the actual amount swapped attached
    Malleable(BoundedMatchResult, Amount),
}

impl ExternalMatch {
    /// Get the bundle ID for an external match
    pub fn bundle_id(&self, nullifier: &Nullifier) -> Result<String, AuthServerError> {
        match self {
            ExternalMatch::Standard(match_result) => {
                let api_match: ApiExternalMatchResult = match_result.clone().into();
                generate_bundle_id(&api_match, nullifier)
            },
            ExternalMatch::Malleable(match_result, _) => {
                let api_match: ApiBoundedMatchResult = match_result.clone().into();
                generate_malleable_bundle_id(&api_match, nullifier)
            },
        }
    }

    /// Get the external match result from the match bundle
    pub fn match_result(&self) -> ExternalMatchResult {
        match self {
            ExternalMatch::Standard(match_result) => match_result.clone(),
            ExternalMatch::Malleable(match_result, base_amt) => {
                match_result.to_external_match_result(*base_amt)
            },
        }
    }
}
