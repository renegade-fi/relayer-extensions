//! RFQT helpers

use std::collections::HashMap;

use alloy_primitives::Bytes;
use auth_server_api::rfqt::{
    Consideration, Level, OrderDetails, RfqtLevelsQueryParams, RfqtLevelsResponse,
    RfqtQuoteRequest, RfqtQuoteResponse, TokenAmount, TokenPairLevels,
};
use renegade_api::http::{
    external_match::{
        ASSEMBLE_MALLEABLE_EXTERNAL_MATCH_ROUTE, AssembleExternalMatchRequest,
        AtomicMatchApiBundle, ExternalMatchRequest, ExternalOrder, ExternalQuoteRequest,
        ExternalQuoteResponse, MalleableAtomicMatchApiBundle,
    },
    order_book::GetDepthForAllPairsResponse,
};
use renegade_circuit_types::{fixed_point::FixedPoint, order::OrderSide};
use renegade_common::types::{chain::Chain, token::Token};
use renegade_util::{get_current_time_millis, hex::biguint_to_hex_addr};

use crate::{
    error::AuthServerError,
    server::api_handlers::{external_match::RequestContext, rfqt::MatchBundle},
};

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

/// Check if the query string indicates malleable calldata should be used
pub fn should_use_malleable_calldata(query_str: &str) -> bool {
    query_str.contains("malleableCalldata=true")
}

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

/// Create a quote request from an RFQT quote request
pub fn create_quote_request(
    req: RfqtQuoteRequest,
) -> Result<ExternalQuoteRequest, AuthServerError> {
    let external_order = transform_rfqt_to_external_order(req)?;
    Ok(ExternalQuoteRequest {
        matching_pool: None,   // Will be set by route_quote_req if needed
        relayer_fee_rate: 0.0, // Will be set by preprocess_rfqt_quote_request
        external_order,
    })
}

/// Transform a quote response to an assemble malleable request context
pub fn transform_quote_to_assemble_malleable_ctx(
    quote: ExternalQuoteResponse,
    req_ctx: RequestContext<ExternalQuoteRequest>,
) -> Result<RequestContext<AssembleExternalMatchRequest>, AuthServerError> {
    let assemble_request = AssembleExternalMatchRequest {
        signed_quote: quote.signed_quote,
        do_gas_estimation: false,
        allow_shared: false,
        matching_pool: None,
        relayer_fee_rate: 0.0,
        receiver_address: None,
        updated_order: None,
    };
    let assemble_quote_request_ctx = RequestContext {
        path: ASSEMBLE_MALLEABLE_EXTERNAL_MATCH_ROUTE.to_string(),
        query_str: req_ctx.query_str,
        user: req_ctx.user,
        sdk_version: req_ctx.sdk_version,
        headers: req_ctx.headers,
        body: assemble_request,
        request_id: req_ctx.request_id,
        key_id: req_ctx.key_id,
        sponsorship_info: None,
    };
    Ok(assemble_quote_request_ctx)
}

/// Create a direct match request from an RFQT quote request
pub fn create_direct_match_request(
    req: RfqtQuoteRequest,
) -> Result<ExternalMatchRequest, AuthServerError> {
    let external_order = transform_rfqt_to_external_order(req.clone())?;
    let receiver_address = Some(req.taker);
    Ok(ExternalMatchRequest {
        do_gas_estimation: false,
        matching_pool: None,   // Will be set by route_direct_match_req if needed
        relayer_fee_rate: 0.0, // Will be set by preprocess_rfqt_quote_request
        receiver_address,
        external_order,
    })
}

/// Transform an RFQT quote request to an external order
fn transform_rfqt_to_external_order(
    req: RfqtQuoteRequest,
) -> Result<ExternalOrder, AuthServerError> {
    // Determine which token is USDC
    let usdc_address = Token::usdc().get_addr_biguint();
    let maker_is_usdc = req.maker_token == usdc_address;
    let taker_is_usdc = req.taker_token == usdc_address;

    if !maker_is_usdc && !taker_is_usdc {
        return Err(AuthServerError::bad_request("Either maker or taker token must be USDC"));
    }

    // Route to appropriate handler based on USDC position
    if taker_is_usdc { transform_taker_usdc_order(req) } else { transform_maker_usdc_order(req) }
}

/// Transform RFQT request when taker token is USDC
/// Internally, we treat this as a buy order since taker sends USDC, maker sends
/// base token
fn transform_taker_usdc_order(req: RfqtQuoteRequest) -> Result<ExternalOrder, AuthServerError> {
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

    Ok(external_order)
}

/// Transform RFQT request when maker token is USDC
/// Internally, we treat this as a sell order since maker sends USDC, taker
/// sends base token
fn transform_maker_usdc_order(req: RfqtQuoteRequest) -> Result<ExternalOrder, AuthServerError> {
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

    Ok(external_order)
}

/// Transform a malleable match bundle into an RFQT quote response
fn transform_malleable_bundle_to_rfqt_response(
    bundle: MalleableAtomicMatchApiBundle,
    rfqt: &RfqtQuoteRequest,
) -> Result<RfqtQuoteResponse, AuthServerError> {
    let maker = bundle
        .settlement_tx
        .to
        .as_ref()
        .and_then(|addr| addr.to().copied())
        .map(|addr| format!("{:#x}", addr))
        .ok_or_else(|| AuthServerError::serde("Missing maker address in settlement transaction"))?;
    let calldata = bundle
        .settlement_tx
        .input
        .input()
        .cloned()
        .ok_or_else(|| AuthServerError::serde("Missing settlement transaction input"))?;

    Ok(build_rfqt_quote_response(
        rfqt,
        bundle.max_receive.mint.clone(),
        bundle.max_receive.amount.to_string(),
        bundle.max_send.mint.clone(),
        bundle.max_send.amount.to_string(),
        maker,
        calldata,
        bundle.match_result.price_fp,
    ))
}

/// Transform a direct match bundle into an RFQT quote response
fn transform_direct_bundle_to_rfqt_response(
    bundle: AtomicMatchApiBundle,
    rfqt: &RfqtQuoteRequest,
) -> Result<RfqtQuoteResponse, AuthServerError> {
    let maker = bundle
        .settlement_tx
        .to
        .as_ref()
        .and_then(|addr| addr.to().copied())
        .map(|addr| format!("{:#x}", addr))
        .ok_or_else(|| AuthServerError::serde("Missing maker address in settlement transaction"))?;
    let calldata = bundle
        .settlement_tx
        .input
        .input()
        .cloned()
        .ok_or_else(|| AuthServerError::serde("Missing settlement transaction input"))?;

    Ok(build_rfqt_quote_response(
        rfqt,
        bundle.receive.mint.clone(),
        bundle.receive.amount.to_string(),
        bundle.send.mint.clone(),
        bundle.send.amount.to_string(),
        maker,
        calldata,
        FixedPoint::from(0u64),
    ))
}

/// Transform a Direct or Malleable match bundle into an RFQT quote response
pub fn transform_match_bundle_to_rfqt_response(
    bundle: MatchBundle,
    rfqt: &RfqtQuoteRequest,
) -> Result<RfqtQuoteResponse, AuthServerError> {
    match bundle {
        MatchBundle::Malleable(bundle) => transform_malleable_bundle_to_rfqt_response(bundle, rfqt),
        MatchBundle::Direct(bundle) => transform_direct_bundle_to_rfqt_response(bundle, rfqt),
    }
}

#[allow(clippy::too_many_arguments)]
/// Build an RFQT quote response
fn build_rfqt_quote_response(
    rfqt: &RfqtQuoteRequest,
    maker_token_addr: String,
    maker_amount: String,
    taker_token_addr: String,
    taker_amount: String,
    maker: String,
    calldata: Bytes,
    price_fp: FixedPoint,
) -> RfqtQuoteResponse {
    let deadline = get_deadline();

    let permitted = TokenAmount { token: maker_token_addr, amount: maker_amount };
    let consideration = Consideration {
        token: taker_token_addr,
        amount: taker_amount,
        counterparty: rfqt.taker.clone(),
        partial_fill_allowed: rfqt.partial_fill_allowed,
    };

    let order = OrderDetails {
        permitted,
        spender: rfqt.spender.clone(),
        nonce: rfqt.nonce.clone(),
        deadline: deadline.to_string(),
        consideration,
    };

    let fee_token = biguint_to_hex_addr(&rfqt.fee_token);

    RfqtQuoteResponse {
        order,
        signature: SIGNATURE.to_string(),
        fee_token,
        fee_amount_bps: rfqt.fee_amount_bps.to_string(),
        fee_token_conversion_rate: rfqt.fee_token_conversion_rate.to_string(),
        maker,
        calldata,
        price_fp,
    }
}

/// Get the deadline for the RFQT order
fn get_deadline() -> u64 {
    (get_current_time_millis() / 1000) + DEADLINE_OFFSET_SECONDS
}
