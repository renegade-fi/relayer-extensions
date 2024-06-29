//! The API for the compliance server

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
#![deny(unsafe_code)]
#![deny(clippy::needless_pass_by_value)]
#![deny(clippy::needless_pass_by_ref_mut)]

use serde::{Deserialize, Serialize};

/// The API endpoint for screening an address for compliance
pub const WALLET_SCREEN_PATH: &str = "/v0/check-compliance";

/// The response type for a compliance check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceCheckResponse {
    /// The compliance status of the wallet
    pub compliance_status: ComplianceStatus,
}

/// The status on compliance for a wallet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ComplianceStatus {
    /// The wallet is compliant
    Compliant,
    /// The wallet is not compliant
    #[allow(missing_docs)]
    NotCompliant { reason: String },
}
