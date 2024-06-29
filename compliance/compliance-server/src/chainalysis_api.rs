//! Helpers for interacting with the chainalysis API

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{db::ComplianceEntry, error::ComplianceServerError};

// -------------
// | API Types |
// -------------

/// The base URL for the chainalysis entities API
const CHAINALYSIS_API_BASE: &str = "https://api.chainalysis.com/api/risk/v2/entities";
/// The header name for the auth token
const TOKEN_HEADER: &str = "Token";

/// The register address request body
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterAddressRequest {
    /// The address to register
    pub address: String,
}

/// The response to a risk assessment query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessmentResponse {
    /// The address that the assessment is for
    pub address: String,
    /// The risk assessment status
    pub risk: String,
    /// The reason for the risk assessment
    #[serde(rename = "riskReason")]
    pub risk_reason: Option<String>,
}

impl RiskAssessmentResponse {
    /// Get a compliance entry from the risk assessment
    pub fn as_compliance_entry(self) -> ComplianceEntry {
        // We allow low and medium risk entries, high and severe are marked
        // non-compliant
        let compliant = match self.risk.as_str() {
            "Low" | "Medium" => true,
            "High" | "Severe" => false,
            x => {
                // For now we don't block on an unknown assessment, this should be unreachable
                warn!("Unexpected risk assessment: {x}");
                true
            },
        };

        let risk_reason = self.risk_reason.unwrap_or_default();
        ComplianceEntry::new(self.address, compliant, self.risk, risk_reason)
    }
}

// ---------------
// | Client Impl |
// ---------------

/// Query chainalysis for the compliance status of a wallet
pub async fn query_chainalysis(
    wallet_address: &str,
    chainalysis_api_key: &str,
) -> Result<ComplianceEntry, ComplianceServerError> {
    // 1. Register the wallet
    register_addr(wallet_address, chainalysis_api_key).await?;

    // 2. Query the risk assessment
    let risk_assessment = query_risk_assessment(wallet_address, chainalysis_api_key).await?;
    Ok(risk_assessment.as_compliance_entry())
}

/// Register a wallet with chainalysis
async fn register_addr(
    wallet_address: &str,
    chainalysis_api_key: &str,
) -> Result<(), ComplianceServerError> {
    let body = RegisterAddressRequest { address: wallet_address.to_string() };
    let client = reqwest::Client::new();
    client
        .post(CHAINALYSIS_API_BASE)
        .header(TOKEN_HEADER, chainalysis_api_key)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

/// Query the risk assessment from chainalysis
async fn query_risk_assessment(
    wallet_address: &str,
    chainalysis_api_key: &str,
) -> Result<RiskAssessmentResponse, ComplianceServerError> {
    let url = format!("{CHAINALYSIS_API_BASE}/{wallet_address}");
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header(TOKEN_HEADER, chainalysis_api_key)
        .send()
        .await?
        .error_for_status()?;

    let risk_assessment: RiskAssessmentResponse = resp.json().await?;
    Ok(risk_assessment)
}
