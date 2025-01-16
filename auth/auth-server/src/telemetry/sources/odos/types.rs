use super::{client::OdosConfig, error::OdosError};
use serde::{Deserialize, Serialize};

/// Request structure for the Odos API quote endpoint.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OdosQuoteRequest {
    pub chain_id: u64,
    pub input_tokens: Vec<InputToken>,
    pub output_tokens: Vec<OutputToken>,
    pub slippage_limit_percent: f64,
    pub disable_rfqs: bool,
}

/// Response structure from the Odos API quote endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OdosQuoteResponse {
    pub in_amounts: Vec<String>,
    pub in_tokens: Vec<String>,
    pub in_values: Vec<f64>,
    pub out_amounts: Vec<String>,
    pub out_tokens: Vec<String>,
    pub out_values: Vec<f64>,
    pub net_out_value: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InputToken {
    pub token_address: String,
    pub amount: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OutputToken {
    pub token_address: String,
    pub proportion: f32,
}

impl OdosQuoteRequest {
    /// Creates a new `OdosQuoteRequest` with the given configuration and
    /// tokens.
    pub fn new(
        config: &OdosConfig,
        input_token: String,
        input_amount: u128,
        output_token: String,
    ) -> Self {
        Self {
            chain_id: config.chain_id,
            input_tokens: vec![InputToken {
                token_address: input_token,
                amount: input_amount.to_string(),
            }],
            output_tokens: vec![OutputToken { token_address: output_token, proportion: 1.0 }],
            slippage_limit_percent: config.slippage_limit_percent,
            disable_rfqs: config.disable_rfqs,
        }
    }
}

impl OdosQuoteResponse {
    /// Gets the input amount from the first token in the quote.
    pub fn get_in_amount(&self) -> Result<u128, OdosError> {
        self.in_amounts
            .first()
            .ok_or_else(|| OdosError::Input("No input amount available".to_string()))?
            .parse()
            .map_err(|e| OdosError::Input(format!("Failed to parse input amount: {}", e)))
    }

    /// Gets the output amount from the first token in the quote.
    pub fn get_out_amount(&self) -> Result<u128, OdosError> {
        self.out_amounts
            .first()
            .ok_or_else(|| OdosError::Input("No output amount available".to_string()))?
            .parse()
            .map_err(|e| OdosError::Input(format!("Failed to parse output amount: {}", e)))
    }

    /// Gets the input token from the first token in the quote.
    pub fn get_in_token(&self) -> Result<String, OdosError> {
        self.in_tokens
            .first()
            .ok_or_else(|| OdosError::Input("No input token available".to_string()))
            .map(|token| token.to_string())
    }

    /// Gets the output token from the first token in the quote.
    pub fn get_out_token(&self) -> Result<String, OdosError> {
        self.out_tokens
            .first()
            .ok_or_else(|| OdosError::Input("No output token available".to_string()))
            .map(|token| token.to_string())
    }
}
