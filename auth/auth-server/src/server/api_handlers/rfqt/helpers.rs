//! RFQT helpers

use std::collections::HashMap;

use auth_server_api::rfqt::{
    Consideration, Level, OrderDetails, RfqtLevelsQueryParams, RfqtLevelsResponse,
    RfqtQuoteRequest, RfqtQuoteResponse, TokenAmount, TokenPairLevels,
};
use renegade_api::http::order_book::GetDepthForAllPairsResponse;
use renegade_common::types::{chain::Chain, token::Token};

use crate::error::AuthServerError;

/// Parse query string into `RfqtLevelsQueryParams` with validation against
/// server chain
pub fn parse_levels_query_params(
    query_str: &str,
    server_chain: Chain,
) -> Result<RfqtLevelsQueryParams, AuthServerError> {
    if query_str.is_empty() {
        return Ok(RfqtLevelsQueryParams::default());
    }

    // Parse chain ID, return bad request on failure
    let chain_id = query_str
        .parse::<u64>()
        .map_err(|_| AuthServerError::bad_request("Invalid chain ID format"))?;

    // Validate chain ID matches server chain
    validate_chain_id(chain_id, server_chain)?;

    Ok(RfqtLevelsQueryParams { chain_id: Some(chain_id) })
}

/// Validate that the provided chain ID matches the server's configured chain
fn validate_chain_id(provided_chain_id: u64, server_chain: Chain) -> Result<(), AuthServerError> {
    let server_chain_id = chain_to_chain_id(server_chain);
    if provided_chain_id != server_chain_id {
        return Err(AuthServerError::bad_request(format!(
            "Chain ID mismatch: expected {server_chain_id}, got {provided_chain_id}",
        )));
    }
    Ok(())
}

/// Convert a Chain enum to its numeric chain ID
fn chain_to_chain_id(chain: Chain) -> u64 {
    match chain {
        Chain::ArbitrumOne => 42161,
        Chain::ArbitrumSepolia => 421614,
        Chain::BaseMainnet => 8453,
        Chain::BaseSepolia => 84532,
        Chain::Devnet => 0,
    }
}

/// Parse request body into `RfqtQuoteRequest`
pub fn parse_quote_request(body: &[u8]) -> Result<RfqtQuoteRequest, AuthServerError> {
    serde_json::from_slice(body).map_err(AuthServerError::serde)
}

/// Deserialize order book depth response from bytes
pub fn deserialize_depth_response(
    body: &[u8],
) -> Result<GetDepthForAllPairsResponse, AuthServerError> {
    serde_json::from_slice(body).map_err(AuthServerError::serde)
}

/// Transform order book depth data to RFQT levels format
pub fn transform_depth_to_levels(
    depth_response: GetDepthForAllPairsResponse,
) -> RfqtLevelsResponse {
    let mut pairs = HashMap::new();
    let usdc_address = Token::usdc().get_addr();

    for price_and_depth in depth_response.pairs {
        let pair_key = format!("{}/{}", price_and_depth.address, usdc_address);
        let base_token = Token::from_addr(&price_and_depth.address);
        let price = price_and_depth.price;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        // Convert buy side to bids
        if price_and_depth.buy.total_quantity > 0 {
            let buy_amount_decimal =
                base_token.convert_to_decimal(price_and_depth.buy.total_quantity);
            bids.push(Level { price: price.to_string(), amount: buy_amount_decimal.to_string() });
        }

        // Convert sell side to asks
        if price_and_depth.sell.total_quantity > 0 {
            let sell_amount_decimal =
                base_token.convert_to_decimal(price_and_depth.sell.total_quantity);
            asks.push(Level { price: price.to_string(), amount: sell_amount_decimal.to_string() });
        }

        pairs.insert(pair_key, TokenPairLevels { bids, asks });
    }

    RfqtLevelsResponse { pairs }
}

// -------------------------
// | Dummy Response Bodies |
// -------------------------

/// Dummy response body for POST /rfqt/v3/quote
pub fn dummy_quote_response() -> RfqtQuoteResponse {
    RfqtQuoteResponse {
        order: OrderDetails {
            permitted: TokenAmount {
                token: "0x514910771af9ca656af840dff83e8264ecf986ca".to_string(),
                amount: "1100000006".to_string(),
            },
            spender: "0x7966af62034313d87ede39380bf60f1a84c62be7".to_string(),
            nonce: "40965050227042607011257170245709898174942929758885760071848663177298536562693".to_string(),
            deadline: "1711125773".to_string(),
            consideration: Consideration {
                token: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".to_string(),
                amount: "1000000000".to_string(),
                counterparty: "0x003e1cb9314926ae6d32479e93541b0ddc8d5de8".to_string(),
                partial_fill_allowed: false,
            },
        },
        signature: "0x81948c4243e0e3a9955ebbc3e7b0223623499f32e90a770387aa41c93c08b5ab196c8e062a368799f458d5e3d88124978cb5a392fd97e8554379904a031a9fbd1b".to_string(),
        fee_token: "0x514910771af9ca656af840dff83e8264ecf986ca".to_string(),
        fee_amount_bps: "3.14".to_string(),
        fee_token_conversion_rate: "13.70".to_string(),
        maker: "0x135e1cb9314926ae6d32479e93541b0ddc8d5de8".to_string(),
    }
}
