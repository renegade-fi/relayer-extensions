//! RFQT helpers

use std::collections::HashMap;

use auth_server_api::rfqt::{
    Consideration, Level, OrderDetails, RfqtLevelsQueryParams, RfqtLevelsResponse,
    RfqtQuoteRequest, RfqtQuoteResponse, TokenAmount, TokenPairLevels,
};
use renegade_api::http::{
    external_match::{ExternalMatchRequest, ExternalMatchResponse, ExternalOrder},
    order_book::GetDepthForAllPairsResponse,
};
use renegade_circuit_types::order::OrderSide;
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

/// Transform order book depth data to RFQT levels format
pub fn transform_depth_to_levels(
    depth_response: GetDepthForAllPairsResponse,
) -> RfqtLevelsResponse {
    let mut pairs = HashMap::new();

    for price_and_depth in depth_response.pairs {
        let pair_key = format!("{}/{}", price_and_depth.address, Token::usdc().get_addr());
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

/// Transform an RFQT quote request to an external match request
pub fn transform_rfqt_to_external_match_request(
    req: RfqtQuoteRequest,
) -> Result<ExternalMatchRequest, AuthServerError> {
    // Determine which token is USDC
    let usdc_address = Token::usdc().get_addr_biguint();
    let maker_is_usdc = &req.maker_token == &usdc_address;
    let taker_is_usdc = &req.taker_token == &usdc_address;

    if !maker_is_usdc && !taker_is_usdc {
        return Err(AuthServerError::bad_request("Either maker or taker token must be USDC"));
    }

    // Route to appropriate handler based on USDC position
    if taker_is_usdc {
        transform_taker_usdc_request(req)
    } else {
        transform_maker_usdc_request(req)
    }
}

/// Transform RFQT request when taker token is USDC
/// Internally, we treat this as a buy order since taker sends USDC, maker sends
/// base token
fn transform_taker_usdc_request(
    req: RfqtQuoteRequest,
) -> Result<ExternalMatchRequest, AuthServerError> {
    // Determine which field to set based on provided amounts
    let (base_amount, quote_amount, min_fill_size) = if let Some(amount) = req.taker_amount {
        // Taker amount is USDC (quote token)
        let min_fill_size = if req.partial_fill_allowed { 0 } else { amount };
        (0, amount, min_fill_size)
    } else if let Some(amount) = req.maker_amount {
        // Maker amount is base token
        let min_fill_size = if req.partial_fill_allowed { 0 } else { amount };
        (amount, 0, min_fill_size)
    } else {
        return Err(AuthServerError::bad_request("No amount provided"));
    };

    // Create external order
    let external_order = ExternalOrder {
        base_mint: req.maker_token,
        quote_mint: req.taker_token,
        side: OrderSide::Buy, // Taker is buying base token with USDC
        base_amount,
        quote_amount,
        exact_base_output: 0,
        exact_quote_output: 0,
        min_fill_size,
    };

    Ok(ExternalMatchRequest {
        do_gas_estimation: false,
        matching_pool: None,   // Will be set by route_direct_match_req if needed
        relayer_fee_rate: 0.0, // Will be set by preprocess_rfqt_quote_request
        receiver_address: Some(req.taker),
        external_order,
    })
}

/// Transform RFQT request when maker token is USDC
/// Internally, we treat this as a sell order since maker sends USDC, taker
/// sends base token
fn transform_maker_usdc_request(
    req: RfqtQuoteRequest,
) -> Result<ExternalMatchRequest, AuthServerError> {
    // Determine which field to set based on provided amounts
    let (base_amount, quote_amount, min_fill_size) = if let Some(amount) = req.taker_amount {
        // Taker amount is base token
        let min_fill_size = if req.partial_fill_allowed { 0 } else { amount };
        (amount, 0, min_fill_size)
    } else if let Some(amount) = req.maker_amount {
        // Maker amount is USDC (quote token)
        let min_fill_size = if req.partial_fill_allowed { 0 } else { amount };
        (0, amount, min_fill_size)
    } else {
        return Err(AuthServerError::bad_request("No amount provided"));
    };

    // Create external order
    let external_order = ExternalOrder {
        base_mint: req.taker_token,
        quote_mint: req.maker_token,
        side: OrderSide::Sell, // Taker is selling base token for USDC
        base_amount,
        quote_amount,
        exact_base_output: 0,
        exact_quote_output: 0,
        min_fill_size,
    };

    Ok(ExternalMatchRequest {
        do_gas_estimation: false,
        matching_pool: None,   // Will be set by route_direct_match_req if needed
        relayer_fee_rate: 0.0, // Will be set by preprocess_rfqt_quote_request
        receiver_address: Some(req.taker),
        external_order,
    })
}

/// Transform an external match response to an RFQT quote response
pub fn transform_external_match_to_rfqt_response(
    _external_resp: ExternalMatchResponse,
) -> Result<RfqtQuoteResponse, AuthServerError> {
    // TODO: Extract actual order details from match_bundle
    Ok(dummy_quote_response())
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
