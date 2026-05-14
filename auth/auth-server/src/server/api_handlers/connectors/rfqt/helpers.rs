//! RFQT helpers

use std::collections::HashMap;

use alloy_primitives::{Address, Bytes, TxKind};
use auth_server_api::SponsoredQuoteResponse;
use auth_server_api::rfqt::{
    Consideration, Level, OrderDetails, RfqtLevelsQueryParams, RfqtLevelsResponse,
    RfqtQuoteRequest, RfqtQuoteResponse, TokenAmount, TokenPairLevels,
};
use renegade_circuit_types::Amount;
use renegade_external_api::{
    http::{
        external_match::{
            ASSEMBLE_MATCH_BUNDLE_ROUTE, AssembleExternalMatchRequest, ExternalMatchAssemblyType,
            ExternalMatchingEngineOptions, ExternalQuoteRequest,
        },
        market::GetMarketDepthsResponse,
    },
    types::{BoundedExternalMatchApiBundle, ExternalOrder},
};
use renegade_types_core::{Chain, Token};
use renegade_util::{get_current_time_millis, hex::address_to_hex_string};

use crate::{
    error::AuthServerError,
    server::api_handlers::external_match::RequestContext,
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
/// Returns true by default (malleable calldata enabled), false only when
/// explicitly disabled
pub fn should_use_malleable_calldata(query_str: &str) -> bool {
    !query_str.contains("malleableCalldata=false")
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
        Chain::EthereumMainnet => 1,
        Chain::EthereumSepolia => 11155111,
        Chain::Devnet => 0,
    }
}

/// Transform v2 market-depths data into the RFQT levels response shape.
///
/// v2 `GetMarketDepthsResponse` carries a `MarketInfo` per pair, which already
/// includes a timestamped price, so no per-mint price fan-out is needed.
pub fn transform_depth_to_levels(
    depth_response: GetMarketDepthsResponse,
) -> RfqtLevelsResponse {
    let mut pairs = HashMap::new();
    let usdc_addr = Token::usdc().get_addr();

    for market_depth in depth_response.market_depths {
        let base_addr = address_to_hex_string(&market_depth.market.base.address);
        let pair_key = format!("{base_addr}/{usdc_addr}");
        let base_token = Token::from_addr(&base_addr);
        let price = market_depth.market.price.price;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        // Buy depth -> bids (taker buying base)
        if market_depth.buy.total_quantity > 0 {
            let amount_decimal = base_token.convert_to_decimal(market_depth.buy.total_quantity);
            bids.push(Level { price: price.to_string(), amount: amount_decimal.to_string() });
        }

        // Sell depth -> asks (taker selling base)
        if market_depth.sell.total_quantity > 0 {
            let amount_decimal = base_token.convert_to_decimal(market_depth.sell.total_quantity);
            asks.push(Level { price: price.to_string(), amount: amount_decimal.to_string() });
        }

        pairs.insert(pair_key, TokenPairLevels { bids, asks });
    }

    RfqtLevelsResponse { pairs }
}

/// Create an external quote request from an RFQT quote request
pub fn create_quote_request(
    req: &RfqtQuoteRequest,
) -> Result<ExternalQuoteRequest, AuthServerError> {
    let external_order = transform_rfqt_to_external_order(req)?;
    Ok(ExternalQuoteRequest {
        external_order,
        options: ExternalMatchingEngineOptions::default(),
    })
}

/// Transform a sponsored quote response into an assemble request context
///
/// Builds an `AssembleExternalMatchRequest` with `ExternalMatchAssemblyType::QuotedOrder`
/// pointing at the v2 assemble route. The receiver is left unset; for the
/// malleable RFQT path 0x's Settler picks the receiver at settlement time.
pub fn transform_quote_to_assemble_malleable_ctx(
    quote: SponsoredQuoteResponse,
    req_ctx: RequestContext<ExternalQuoteRequest>,
) -> Result<RequestContext<AssembleExternalMatchRequest>, AuthServerError> {
    let assemble_request = AssembleExternalMatchRequest {
        do_gas_estimation: false,
        receiver_address: None,
        order: ExternalMatchAssemblyType::QuotedOrder {
            signed_quote: quote.signed_quote,
            updated_order: None,
        },
        options: ExternalMatchingEngineOptions::default(),
    };
    Ok(RequestContext {
        path: ASSEMBLE_MATCH_BUNDLE_ROUTE.to_string(),
        query_str: req_ctx.query_str,
        user: req_ctx.user,
        sdk_version: req_ctx.sdk_version,
        headers: req_ctx.headers,
        body: assemble_request,
        request_id: req_ctx.request_id,
        key_id: req_ctx.key_id,
        sponsorship_info: None,
    })
}

/// Create a direct-order assemble request from an RFQT quote request
///
/// v2 has no separate `request-external-match` endpoint; the direct path is
/// modeled as an assemble with `ExternalMatchAssemblyType::DirectOrder`. The
/// receiver is set to the RFQT taker so the byte-fixed direct calldata embeds
/// the correct counterparty.
pub fn create_direct_match_request(
    req: &RfqtQuoteRequest,
) -> Result<AssembleExternalMatchRequest, AuthServerError> {
    let external_order = transform_rfqt_to_external_order(req)?;
    let receiver_address = req
        .taker
        .parse::<Address>()
        .map_err(|_| AuthServerError::bad_request("Invalid taker address"))?;
    Ok(AssembleExternalMatchRequest {
        do_gas_estimation: false,
        receiver_address: Some(receiver_address),
        order: ExternalMatchAssemblyType::DirectOrder { external_order },
        options: ExternalMatchingEngineOptions::default(),
    })
}

/// Transform an RFQT quote request into a v2 `ExternalOrder`
///
/// 0x's RFQT request is symmetric in maker/taker token, matching v2's
/// `input_mint`/`output_mint` orientation directly. USDC-side validation is
/// deferred to `Server::validate_request_body`.
fn transform_rfqt_to_external_order(
    req: &RfqtQuoteRequest,
) -> Result<ExternalOrder, AuthServerError> {
    // Exact-output: maker_amount is fixed; min_fill_size must be 0.
    // Else: enforce all-or-nothing fill unless partial fills are allowed.
    let min_fill_size = if req.maker_amount.is_some() || req.partial_fill_allowed {
        0
    } else {
        req.taker_amount.unwrap_or_default()
    };

    Ok(ExternalOrder {
        input_mint: req.taker_token,
        output_mint: req.maker_token,
        input_amount: req.taker_amount.unwrap_or_default(),
        output_amount: req.maker_amount.unwrap_or_default(),
        use_exact_output_amount: req.maker_amount.is_some(),
        min_fill_size,
    })
}

/// Transform a v2 bounded match bundle into an RFQT quote response.
///
/// `malleable` selects the v1 "malleable" (bounded) response shape vs. the v1
/// "direct" (single-amount) response shape. The underlying v2 bundle is the
/// same `BoundedExternalMatchApiBundle` either way; the difference is whether
/// price and min/max receive/send fields are populated.
pub fn transform_match_bundle_to_rfqt_response(
    bundle: &BoundedExternalMatchApiBundle,
    rfqt: &RfqtQuoteRequest,
    malleable: bool,
) -> Result<RfqtQuoteResponse, AuthServerError> {
    let maker = match &bundle.settlement_tx.to {
        Some(TxKind::Call(addr)) => format!("{addr:#x}"),
        _ => {
            return Err(AuthServerError::serde(
                "Missing maker address in settlement transaction",
            ));
        },
    };
    let calldata = bundle
        .settlement_tx
        .input
        .input()
        .cloned()
        .ok_or_else(|| AuthServerError::serde("Missing settlement transaction input"))?;

    // v2 orients amounts from the external party's perspective:
    //  - max_receive is what the taker receives (maker side of the RFQT order)
    //  - max_send is what the taker sends (taker side of the RFQT order)
    // The maker_amount / taker_amount fields below mirror v1's calldata defaults
    // (which were also computed at the max-input boundary).
    let maker_token = address_to_hex_string(&bundle.match_result.output_mint);
    let taker_token = address_to_hex_string(&bundle.match_result.input_mint);
    let maker_amount = bundle.max_receive.amount;
    let taker_amount = bundle.max_send.amount;

    let (price, max_receive, min_receive, max_send, min_send) = if malleable {
        (
            Some(bundle.match_result.price_fp.to_f64()),
            Some(bundle.max_receive.amount),
            Some(bundle.min_receive.amount),
            Some(bundle.max_send.amount),
            Some(bundle.min_send.amount),
        )
    } else {
        (None, None, None, None, None)
    };

    Ok(build_rfqt_quote_response(
        rfqt,
        maker_token,
        maker_amount,
        taker_token,
        taker_amount,
        maker,
        calldata,
        price,
        max_receive,
        min_receive,
        max_send,
        min_send,
    ))
}

#[allow(clippy::too_many_arguments)]
/// Build an RFQT quote response
fn build_rfqt_quote_response(
    rfqt: &RfqtQuoteRequest,
    maker_token_addr: String,
    maker_amount: Amount,
    taker_token_addr: String,
    taker_amount: Amount,
    maker: String,
    calldata: Bytes,
    price: Option<f64>,
    max_receive: Option<Amount>,
    min_receive: Option<Amount>,
    max_send: Option<Amount>,
    min_send: Option<Amount>,
) -> RfqtQuoteResponse {
    let deadline = get_deadline();

    let permitted = TokenAmount { token: maker_token_addr, amount: maker_amount.to_string() };
    let consideration = Consideration {
        token: taker_token_addr,
        amount: taker_amount.to_string(),
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

    let fee_token = address_to_hex_string(&rfqt.fee_token);

    RfqtQuoteResponse {
        order,
        signature: SIGNATURE.to_string(),
        fee_token,
        fee_amount_bps: rfqt.fee_amount_bps.to_string(),
        fee_token_conversion_rate: rfqt.fee_token_conversion_rate.to_string(),
        maker,
        calldata,
        price,
        max_taker_receive: max_receive,
        min_taker_receive: min_receive,
        max_taker_send: max_send,
        min_taker_send: min_send,
    }
}

/// Get the deadline for the RFQT order
fn get_deadline() -> u64 {
    (get_current_time_millis() / 1000) + DEADLINE_OFFSET_SECONDS
}

#[cfg(test)]
mod tests {
    use std::sync::Once;

    use alloy::rpc::types::TransactionRequest;
    use renegade_circuit_types::fixed_point::FixedPoint;
    use renegade_external_api::types::{
        ApiBoundedMatchResult, ApiExternalAssetTransfer, FeeTakeRate,
    };
    use renegade_types_core::{
        USDC_TICKER, set_default_chain, write_token_decimals_map, write_token_remaps,
    };

    use super::*;

    // ---------- fixtures ----------

    /// USDC address used across fixtures (real Arbitrum One USDC)
    const USDC_ADDR: &str = "0xaf88d065e77c8cc2239327c5edb3a432268e5831";
    /// WETH address used across fixtures (real Arbitrum One WETH)
    const WETH_ADDR: &str = "0x82af49447d8a07e3bd95bd0d56f35241523fbab1";
    /// Generic taker address used for `RfqtQuoteRequest::taker` / receivers
    const TAKER_ADDR: &str = "0x1111111111111111111111111111111111111111";
    /// Generic maker address used as the `to` of the settlement transaction
    const MAKER_ADDR: &str = "0x2222222222222222222222222222222222222222";

    static TOKEN_REMAP_INIT: Once = Once::new();

    /// Idempotently populate `TOKEN_REMAPS_BY_CHAIN` and `DECIMALS_BY_CHAIN`
    /// for Arbitrum One so `Token::usdc()` / `Token::from_addr` /
    /// `Token::convert_to_decimal` resolve in tests.
    fn setup_token_remap() {
        TOKEN_REMAP_INIT.call_once(|| {
            set_default_chain(Chain::ArbitrumOne);

            let mut remaps = write_token_remaps();
            let map = remaps.entry(Chain::ArbitrumOne).or_default();
            map.insert(USDC_ADDR.to_string(), USDC_TICKER.to_string());
            map.insert(WETH_ADDR.to_string(), "WETH".to_string());

            let mut decimals = write_token_decimals_map();
            let dmap = decimals.entry(Chain::ArbitrumOne).or_default();
            dmap.insert(USDC_ADDR.to_string(), 6);
            dmap.insert(WETH_ADDR.to_string(), 18);
        });
    }

    /// Parse a `&str` into an `Address`; panics on malformed input
    /// (used only in tests).
    fn addr(s: &str) -> Address {
        s.parse().expect("test address")
    }

    /// Build a default `RfqtQuoteRequest` matching the most common 0x shape:
    /// taker sends USDC for WETH (a buy from the taker's perspective).
    fn mock_rfqt_buy_request() -> RfqtQuoteRequest {
        RfqtQuoteRequest {
            chain_id: 42161,
            maker_token: addr(WETH_ADDR),
            taker_token: addr(USDC_ADDR),
            taker_amount: Some(1_000_000), // 1 USDC at 6 decimals
            maker_amount: None,
            taker: TAKER_ADDR.to_string(),
            nonce: "1".to_string(),
            partial_fill_allowed: true,
            spender: "0xspender".to_string(),
            zid: "zid".to_string(),
            app_id: "app".to_string(),
            fee_token: addr(USDC_ADDR),
            fee_amount_bps: 5.0,
            fee_token_conversion_rate: 1.0,
        }
    }

    /// Build a sell-flavored RFQT request: taker sends WETH for USDC.
    fn mock_rfqt_sell_request() -> RfqtQuoteRequest {
        RfqtQuoteRequest {
            maker_token: addr(USDC_ADDR),
            taker_token: addr(WETH_ADDR),
            taker_amount: Some(1_000_000_000_000_000_000), // 1 WETH at 18 decimals
            maker_amount: None,
            ..mock_rfqt_buy_request()
        }
    }

    /// Build a `BoundedExternalMatchApiBundle` for a taker-buys-WETH match
    /// (input = USDC, output = WETH).
    fn mock_buy_bundle() -> BoundedExternalMatchApiBundle {
        // 1 USDC -> 0.0005 WETH at ~2000 USDC/WETH (price expressed in
        // external-party output-per-input units, i.e. WETH-per-USDC).
        let price_fp = FixedPoint::from_f64_round_down(0.0005);
        let settlement_tx = TransactionRequest::default()
            .to(addr(MAKER_ADDR))
            .input(alloy::rpc::types::TransactionInput::new(Bytes::from(vec![
                0xde, 0xad, 0xbe, 0xef,
            ])));

        BoundedExternalMatchApiBundle {
            match_result: ApiBoundedMatchResult {
                input_mint: addr(USDC_ADDR),
                output_mint: addr(WETH_ADDR),
                price_fp,
                min_input_amount: 500_000,
                max_input_amount: 1_000_000,
            },
            fee_rates: FeeTakeRate {
                relayer_fee_rate: FixedPoint::zero(),
                protocol_fee_rate: FixedPoint::zero(),
            },
            max_receive: ApiExternalAssetTransfer {
                mint: addr(WETH_ADDR),
                amount: 500_000_000_000_000, // 0.0005 WETH
            },
            min_receive: ApiExternalAssetTransfer {
                mint: addr(WETH_ADDR),
                amount: 250_000_000_000_000,
            },
            max_send: ApiExternalAssetTransfer { mint: addr(USDC_ADDR), amount: 1_000_000 },
            min_send: ApiExternalAssetTransfer { mint: addr(USDC_ADDR), amount: 500_000 },
            settlement_tx,
            deadline: 0,
        }
    }

    // ---------- pure helpers ----------

    #[test]
    fn malleable_default_on() {
        assert!(should_use_malleable_calldata(""));
        assert!(should_use_malleable_calldata("foo=bar"));
        assert!(should_use_malleable_calldata("malleableCalldata=true"));
    }

    #[test]
    fn malleable_off_when_explicitly_disabled() {
        assert!(!should_use_malleable_calldata("malleableCalldata=false"));
        assert!(!should_use_malleable_calldata("other=1&malleableCalldata=false"));
    }

    #[test]
    fn chain_to_chain_id_covers_all_variants() {
        assert_eq!(chain_to_chain_id(Chain::ArbitrumOne), 42161);
        assert_eq!(chain_to_chain_id(Chain::ArbitrumSepolia), 421614);
        assert_eq!(chain_to_chain_id(Chain::BaseMainnet), 8453);
        assert_eq!(chain_to_chain_id(Chain::BaseSepolia), 84532);
        assert_eq!(chain_to_chain_id(Chain::EthereumMainnet), 1);
        assert_eq!(chain_to_chain_id(Chain::EthereumSepolia), 11155111);
        assert_eq!(chain_to_chain_id(Chain::Devnet), 0);
    }

    #[test]
    fn parse_levels_query_params_empty_returns_default() {
        let p = parse_levels_query_params("", Chain::ArbitrumOne).unwrap();
        assert!(p.chain_id.is_none());
    }

    #[test]
    fn parse_levels_query_params_matching_chain_accepted() {
        let p = parse_levels_query_params("42161", Chain::ArbitrumOne).unwrap();
        assert_eq!(p.chain_id, Some(42161));
    }

    #[test]
    fn parse_levels_query_params_mismatched_chain_rejected() {
        let err = parse_levels_query_params("1", Chain::ArbitrumOne).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("Chain ID mismatch"), "unexpected error: {msg}");
    }

    #[test]
    fn parse_levels_query_params_malformed_rejected() {
        let err = parse_levels_query_params("not-a-number", Chain::ArbitrumOne).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("Invalid chain ID"), "unexpected error: {msg}");
    }

    // ---------- request transforms ----------

    #[test]
    fn buy_request_maps_to_v2_external_order() {
        let req = mock_rfqt_buy_request();
        let order = transform_rfqt_to_external_order(&req).unwrap();

        assert_eq!(order.input_mint, addr(USDC_ADDR));
        assert_eq!(order.output_mint, addr(WETH_ADDR));
        assert_eq!(order.input_amount, 1_000_000);
        assert_eq!(order.output_amount, 0);
        assert!(!order.use_exact_output_amount);
        // partial_fill_allowed=true on the fixture, so min_fill_size should be 0.
        assert_eq!(order.min_fill_size, 0);
    }

    #[test]
    fn sell_request_maps_to_v2_external_order() {
        let req = mock_rfqt_sell_request();
        let order = transform_rfqt_to_external_order(&req).unwrap();

        assert_eq!(order.input_mint, addr(WETH_ADDR));
        assert_eq!(order.output_mint, addr(USDC_ADDR));
        assert_eq!(order.input_amount, 1_000_000_000_000_000_000);
        assert!(!order.use_exact_output_amount);
    }

    #[test]
    fn exact_output_request_sets_use_exact_output_amount() {
        let mut req = mock_rfqt_buy_request();
        req.taker_amount = None;
        req.maker_amount = Some(123_456); // exact-output: target 123_456 WETH atoms
        let order = transform_rfqt_to_external_order(&req).unwrap();

        assert!(order.use_exact_output_amount);
        assert_eq!(order.output_amount, 123_456);
        assert_eq!(order.input_amount, 0);
        // Exact-output orders force min_fill_size = 0 regardless of partial.
        assert_eq!(order.min_fill_size, 0);
    }

    #[test]
    fn no_partial_fill_sets_min_fill_size_to_taker_amount() {
        let mut req = mock_rfqt_buy_request();
        req.partial_fill_allowed = false;
        let order = transform_rfqt_to_external_order(&req).unwrap();
        assert_eq!(order.min_fill_size, 1_000_000);
    }

    #[test]
    fn create_quote_request_wraps_external_order_with_default_options() {
        let req = mock_rfqt_buy_request();
        let q = create_quote_request(&req).unwrap();
        assert_eq!(q.external_order.input_mint, addr(USDC_ADDR));
        assert!(q.options.relayer_fee_rate.is_none());
        assert!(q.options.matching_pool.is_none());
    }

    #[test]
    fn create_direct_match_request_parses_taker_and_sets_receiver() {
        let req = mock_rfqt_buy_request();
        let assemble = create_direct_match_request(&req).unwrap();

        assert_eq!(assemble.receiver_address, Some(addr(TAKER_ADDR)));
        assert!(!assemble.do_gas_estimation);
        match assemble.order {
            ExternalMatchAssemblyType::DirectOrder { external_order } => {
                assert_eq!(external_order.input_mint, addr(USDC_ADDR));
                assert_eq!(external_order.output_mint, addr(WETH_ADDR));
            },
            _ => panic!("expected DirectOrder variant"),
        }
    }

    #[test]
    fn create_direct_match_request_rejects_malformed_taker() {
        let mut req = mock_rfqt_buy_request();
        req.taker = "not-an-address".to_string();
        let err = create_direct_match_request(&req).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("Invalid taker"), "unexpected error: {msg}");
    }

    // ---------- assemble-context transform ----------

    #[test]
    fn malleable_assemble_ctx_uses_quoted_order_and_v2_route() {
        // Build a minimal SponsoredQuoteResponse. We don't decode the
        // signed_quote internally, so a default ApiSignedQuote is fine.
        use renegade_external_api::types::{
            ApiExternalMatchResult, ApiExternalQuote, ApiSignedQuote, ApiTimestampedPrice,
            ApiTimestampedPriceFp,
        };
        let signed_quote = ApiSignedQuote {
            quote: ApiExternalQuote {
                order: renegade_external_api::types::ExternalOrder::default(),
                match_result: ApiExternalMatchResult {
                    input_mint: addr(USDC_ADDR),
                    output_mint: addr(WETH_ADDR),
                    input_amount: 0,
                    output_amount: 0,
                    price_fp: ApiTimestampedPriceFp {
                        price: FixedPoint::from_f64_round_down(0.0005),
                        timestamp: 0,
                    },
                },
                fees: Default::default(),
                send: ApiExternalAssetTransfer { mint: addr(USDC_ADDR), amount: 0 },
                receive: ApiExternalAssetTransfer { mint: addr(WETH_ADDR), amount: 0 },
                price: ApiTimestampedPrice::new(FixedPoint::from_f64_round_down(0.0005)),
                timestamp: 0,
            },
            signature: vec![],
            deadline: 0,
        };
        let sponsored = SponsoredQuoteResponse { signed_quote, gas_sponsorship_info: None };

        let req_ctx = RequestContext {
            path: "ignored".to_string(),
            query_str: "q".to_string(),
            user: "u".to_string(),
            key_id: uuid::Uuid::nil(),
            sdk_version: "sdk".to_string(),
            headers: http::HeaderMap::new(),
            body: create_quote_request(&mock_rfqt_buy_request()).unwrap(),
            sponsorship_info: None,
            request_id: uuid::Uuid::nil(),
        };

        let out = transform_quote_to_assemble_malleable_ctx(sponsored, req_ctx).unwrap();
        assert_eq!(out.path, ASSEMBLE_MATCH_BUNDLE_ROUTE);
        assert!(out.body.receiver_address.is_none(), "malleable path leaves receiver unset");
        assert!(!out.body.do_gas_estimation);
        match out.body.order {
            ExternalMatchAssemblyType::QuotedOrder { updated_order, .. } => {
                assert!(updated_order.is_none());
            },
            _ => panic!("expected QuotedOrder variant"),
        }
    }

    // ---------- response transforms ----------

    #[test]
    fn malleable_response_populates_min_max_and_price() {
        let req = mock_rfqt_buy_request();
        let bundle = mock_buy_bundle();
        let resp =
            transform_match_bundle_to_rfqt_response(&bundle, &req, true /* malleable */).unwrap();

        // Maker receives the output mint amounts; taker sends the input mint amounts.
        assert_eq!(resp.order.permitted.token, address_to_hex_string(&addr(WETH_ADDR)));
        assert_eq!(resp.order.permitted.amount, "500000000000000");
        assert_eq!(resp.order.consideration.token, address_to_hex_string(&addr(USDC_ADDR)));
        assert_eq!(resp.order.consideration.amount, "1000000");
        // Maker is the settlement_tx.to address (lower-cased hex).
        assert_eq!(resp.maker, format!("{:#x}", addr(MAKER_ADDR)));
        // Calldata is copied through verbatim.
        assert_eq!(resp.calldata, Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]));
        // Malleable mode populates price + min/max receive/send.
        assert!(resp.price.is_some());
        assert_eq!(resp.max_taker_receive, Some(500_000_000_000_000));
        assert_eq!(resp.min_taker_receive, Some(250_000_000_000_000));
        assert_eq!(resp.max_taker_send, Some(1_000_000));
        assert_eq!(resp.min_taker_send, Some(500_000));
        // Counterparty mirrors the RFQT request taker.
        assert_eq!(resp.order.consideration.counterparty, TAKER_ADDR);
        assert_eq!(resp.order.consideration.partial_fill_allowed, true);
        // Empty signature is preserved.
        assert_eq!(resp.signature, "0x0");
    }

    #[test]
    fn direct_response_strips_optional_fields() {
        let req = mock_rfqt_buy_request();
        let bundle = mock_buy_bundle();
        let resp =
            transform_match_bundle_to_rfqt_response(&bundle, &req, false /* malleable */).unwrap();
        assert!(resp.price.is_none());
        assert!(resp.max_taker_receive.is_none());
        assert!(resp.min_taker_receive.is_none());
        assert!(resp.max_taker_send.is_none());
        assert!(resp.min_taker_send.is_none());
        // Amounts in permitted/consideration still match the max bounds.
        assert_eq!(resp.order.permitted.amount, "500000000000000");
        assert_eq!(resp.order.consideration.amount, "1000000");
    }

    #[test]
    fn response_transform_errors_when_settlement_tx_to_missing() {
        let req = mock_rfqt_buy_request();
        let mut bundle = mock_buy_bundle();
        bundle.settlement_tx.to = None;
        let err = transform_match_bundle_to_rfqt_response(&bundle, &req, true).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("Missing maker address"), "unexpected error: {msg}");
    }

    #[test]
    fn response_transform_errors_when_calldata_missing() {
        let req = mock_rfqt_buy_request();
        let mut bundle = mock_buy_bundle();
        bundle.settlement_tx.input = alloy::rpc::types::TransactionInput::default();
        let err = transform_match_bundle_to_rfqt_response(&bundle, &req, true).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("Missing settlement transaction input"), "unexpected error: {msg}");
    }

    // ---------- depth -> levels ----------

    #[test]
    fn empty_depth_response_produces_empty_levels() {
        setup_token_remap();
        let resp = transform_depth_to_levels(GetMarketDepthsResponse { market_depths: vec![] });
        assert!(resp.pairs.is_empty());
    }

    #[test]
    fn depth_with_both_sides_produces_one_bid_and_one_ask() {
        setup_token_remap();

        use renegade_external_api::types::market::{DepthSide, MarketDepth, MarketInfo};
        use renegade_external_api::types::{ApiToken, ApiTimestampedPrice};

        let market = MarketInfo {
            base: ApiToken { address: addr(WETH_ADDR), symbol: "WETH".to_string() },
            quote: ApiToken { address: addr(USDC_ADDR), symbol: USDC_TICKER.to_string() },
            price: ApiTimestampedPrice { price: 2000.0, timestamp: 0 },
            internal_match_fee_rates: FeeTakeRate {
                relayer_fee_rate: FixedPoint::zero(),
                protocol_fee_rate: FixedPoint::zero(),
            },
            external_match_fee_rates: FeeTakeRate {
                relayer_fee_rate: FixedPoint::zero(),
                protocol_fee_rate: FixedPoint::zero(),
            },
        };
        // 0.0005 WETH on each side.
        let depth = MarketDepth {
            market,
            buy: DepthSide { total_quantity: 500_000_000_000_000, total_quantity_usd: 1.0 },
            sell: DepthSide { total_quantity: 500_000_000_000_000, total_quantity_usd: 1.0 },
        };

        let resp = transform_depth_to_levels(GetMarketDepthsResponse {
            market_depths: vec![depth],
        });

        let expected_key = format!("{WETH_ADDR}/{USDC_ADDR}");
        let levels = resp.pairs.get(&expected_key).expect("pair key present");
        assert_eq!(levels.bids.len(), 1);
        assert_eq!(levels.asks.len(), 1);
        assert_eq!(levels.bids[0].price, "2000");
        assert_eq!(levels.bids[0].amount, "0.0005");
        assert_eq!(levels.asks[0].amount, "0.0005");
    }

    #[test]
    fn zero_side_is_suppressed() {
        setup_token_remap();

        use renegade_external_api::types::market::{DepthSide, MarketDepth, MarketInfo};
        use renegade_external_api::types::{ApiToken, ApiTimestampedPrice};

        let market = MarketInfo {
            base: ApiToken { address: addr(WETH_ADDR), symbol: "WETH".to_string() },
            quote: ApiToken { address: addr(USDC_ADDR), symbol: USDC_TICKER.to_string() },
            price: ApiTimestampedPrice { price: 2000.0, timestamp: 0 },
            internal_match_fee_rates: FeeTakeRate {
                relayer_fee_rate: FixedPoint::zero(),
                protocol_fee_rate: FixedPoint::zero(),
            },
            external_match_fee_rates: FeeTakeRate {
                relayer_fee_rate: FixedPoint::zero(),
                protocol_fee_rate: FixedPoint::zero(),
            },
        };
        let depth = MarketDepth {
            market,
            buy: DepthSide { total_quantity: 0, total_quantity_usd: 0.0 },
            sell: DepthSide { total_quantity: 500_000_000_000_000, total_quantity_usd: 1.0 },
        };

        let resp = transform_depth_to_levels(GetMarketDepthsResponse {
            market_depths: vec![depth],
        });
        let levels = resp.pairs.values().next().unwrap();
        assert!(levels.bids.is_empty());
        assert_eq!(levels.asks.len(), 1);
    }
}
