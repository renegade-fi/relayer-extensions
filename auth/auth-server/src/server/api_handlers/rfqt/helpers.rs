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
use renegade_util::{get_current_time_millis, hex::biguint_to_hex_addr};

use crate::error::AuthServerError;

// -------------
// | Constants |
// -------------

/// The number of seconds to add to the current time to get the deadline on the
/// permit signature.
///
/// Specifically required to be greater than 60 seconds from
/// the current time, although unused since we are not a traditional market
/// maker and don't use permits.
const DEADLINE_OFFSET_SECONDS: u64 = 70;

/// This is the signed permit message. As we don't use permits, this is just an
/// empty string.
const SIGNATURE: &str = "0x0";

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
    let maker_is_usdc = req.maker_token == usdc_address;
    let taker_is_usdc = req.taker_token == usdc_address;

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
    let min_fill_size = match req.maker_amount {
        Some(_) => 0, // Exact-output order: min_fill_size must be 0
        None => {
            if req.partial_fill_allowed {
                0
            } else {
                req.taker_amount.unwrap_or_default()
            }
        },
    };

    let external_order = ExternalOrder {
        base_mint: req.maker_token,
        quote_mint: req.taker_token,
        side: OrderSide::Buy, // Taker is buying base token with USDC
        base_amount: 0,
        quote_amount: req.taker_amount.unwrap_or_default(),
        exact_base_output: req.maker_amount.unwrap_or_default(),
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
    let min_fill_size = match req.maker_amount {
        Some(_) => 0, // Exact-output order: min_fill_size must be 0
        None => {
            if req.partial_fill_allowed {
                0
            } else {
                req.taker_amount.unwrap_or_default()
            }
        },
    };

    let external_order = ExternalOrder {
        base_mint: req.taker_token,
        quote_mint: req.maker_token,
        side: OrderSide::Sell, // Taker is selling base token for USDC
        base_amount: req.taker_amount.unwrap_or_default(),
        quote_amount: 0,
        exact_base_output: 0,
        exact_quote_output: req.maker_amount.unwrap_or_default(),
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
    external_match: &ExternalMatchResponse,
    rfqt: RfqtQuoteRequest,
) -> Result<RfqtQuoteResponse, AuthServerError> {
    let bundle = &external_match.match_bundle;
    let maker_token_addr = bundle.receive.mint.clone();
    let maker_amount = bundle.receive.amount;
    let taker_token_addr = bundle.send.mint.clone();
    let taker_amount = bundle.send.amount;

    // Extract maker address from settlement transaction
    let settlement_tx_to = &bundle.settlement_tx.to;
    let maybe_maker_address = settlement_tx_to.as_ref().and_then(|addr| addr.to());
    let maker = maybe_maker_address
        .map(|addr| format!("{:#x}", addr))
        .ok_or(AuthServerError::serde("Missing maker address in settlement transaction"))?;

    // Extract calldata from settlement transaction
    let calldata = bundle
        .settlement_tx
        .input
        .input()
        .ok_or(AuthServerError::serde("Missing settlement transaction input"))?
        .clone();

    // Calculate deadline
    let deadline = get_deadline();

    // Build permitted token amount
    let permitted = TokenAmount { token: maker_token_addr, amount: maker_amount.to_string() };

    // Build consideration
    let consideration = Consideration {
        token: taker_token_addr,
        amount: taker_amount.to_string(),
        counterparty: rfqt.taker,
        partial_fill_allowed: rfqt.partial_fill_allowed,
    };

    // Build order details
    let order = OrderDetails {
        permitted,
        spender: rfqt.spender,
        nonce: rfqt.nonce,
        deadline: deadline.to_string(),
        consideration,
    };

    // Build fee-related fields
    let fee_token = biguint_to_hex_addr(&rfqt.fee_token);

    Ok(RfqtQuoteResponse {
        order,
        signature: SIGNATURE.to_string(),
        fee_token,
        fee_amount_bps: rfqt.fee_amount_bps.to_string(),
        fee_token_conversion_rate: rfqt.fee_token_conversion_rate.to_string(),
        maker,
        calldata,
    })
}

/// Get the deadline for the RFQT order
fn get_deadline() -> u64 {
    (get_current_time_millis() / 1000) + DEADLINE_OFFSET_SECONDS
}
