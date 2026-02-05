//! RFQT endpoints and types

pub mod helpers;
pub mod levels;
pub mod quote;

use crate::server::api_handlers::external_match::RequestContext;
use renegade_external_api::http::external_match::{
    AtomicMatchApiBundle, ExternalMatchRequest, ExternalQuoteRequest, MalleableAtomicMatchApiBundle,
};

/// Request context variants for RFQT flows
pub enum RequestContextVariant {
    /// Context for a malleable external match
    Malleable(RequestContext<ExternalQuoteRequest>),
    /// Context for a normal external match
    Direct(RequestContext<ExternalMatchRequest>),
}

/// Direct or Malleable match bundle
pub enum MatchBundle {
    /// Match bundle for malleable external match
    Malleable(MalleableAtomicMatchApiBundle),
    /// Match bundle for normal external match
    Direct(AtomicMatchApiBundle),
}
