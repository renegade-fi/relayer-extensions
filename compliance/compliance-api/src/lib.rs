use serde::{Deserialize, Serialize};

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
    NotCompliant,
}
