//! RFQT endpoints and types

pub mod helpers;
pub mod levels;
pub mod quote;

use renegade_external_api::http::external_match::{
    AssembleExternalMatchRequest, ExternalQuoteRequest,
};

use crate::server::api_handlers::external_match::RequestContext;

/// Request context variants for RFQT flows
///
/// In v2 the auth-server exposes a single assemble endpoint that handles both
/// quoted (malleable) and direct orders via `ExternalMatchAssemblyType`. The
/// malleable RFQT path still runs a separate quote step first so that gas
/// sponsorship can be applied before assembly, hence the two variants.
#[allow(clippy::large_enum_variant)]
pub enum RequestContextVariant {
    /// Malleable RFQT path: run quote -> sponsor -> assemble.
    Malleable(RequestContext<ExternalQuoteRequest>),
    /// Direct RFQT path: assemble a direct order in a single step.
    Direct(RequestContext<AssembleExternalMatchRequest>),
}
