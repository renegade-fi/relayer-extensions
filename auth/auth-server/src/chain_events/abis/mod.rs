//! ABI helpers for the chain events listener
use renegade_api::http::external_match::{ApiBoundedMatchResult, ApiExternalMatchResult};
use renegade_circuit_types::{
    fees::{FeeTake, FeeTakeRate},
    fixed_point::FixedPoint,
    r#match::{BoundedMatchResult, ExternalMatchResult},
    wallet::Nullifier,
    Amount,
};
use renegade_constants::Scalar;

use crate::{
    bundle_store::helpers::{generate_bundle_id, generate_malleable_bundle_id},
    chain_events::error::OnChainEventListenerError,
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

/// Compute the internal fee take for a standard external match.
///
/// Currently, the internal party is only charged the protocol fee.
pub(crate) fn compute_standard_match_internal_fee_take(
    protocol_fee_scalar: Scalar,
    match_res: &ExternalMatchResult,
) -> FeeTake {
    let protocol_fee_rate = FixedPoint::from_repr(protocol_fee_scalar);
    let internal_fee_rate = FeeTakeRate::new(
        FixedPoint::zero(), // relayer_fee_rate
        protocol_fee_rate,
    );

    compute_internal_fee_take(match_res, internal_fee_rate)
}

/// Compute the internal fee take for a malleable match.
pub(crate) fn compute_malleable_match_internal_fee_take(
    internal_fee_rate: FeeTakeRate,
    bounded_match_res: &BoundedMatchResult,
    base_amt: Amount,
) -> Result<FeeTake, OnChainEventListenerError> {
    let external_match_res = bounded_match_res.to_external_match_result(base_amt);
    Ok(compute_internal_fee_take(&external_match_res, internal_fee_rate))
}

/// Compute the internal fee take for any external match,
/// given the match result and fee take rate.
fn compute_internal_fee_take(
    match_res: &ExternalMatchResult,
    internal_fee_rate: FeeTakeRate,
) -> FeeTake {
    let (_, internal_party_recv) = match_res.external_party_send();
    internal_fee_rate.compute_fee_take(internal_party_recv)
}
