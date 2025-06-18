//! Metric names and tags

// ----------------
// | METRIC NAMES |
// ----------------

/// Metric describing the ratio of matched quote amount to requested quote
/// amount for quotes and matches
pub const EXTERNAL_MATCH_FILL_RATIO: &str = "external_match.fill_ratio";

/// Metric describing the number of external quote requests
pub const EXTERNAL_MATCH_QUOTE_REQUEST_COUNT: &str = "num_external_match_quote_requests";
/// Metric describing the number of external matches requested
pub const NUM_EXTERNAL_MATCH_REQUESTS: &str = "num_external_match_requests";

/// Metric describing the volume of the base asset in an external order request
pub const EXTERNAL_ORDER_BASE_VOLUME: &str = "external_order_base_volume";
/// Metric describing the volume of the quote asset in an external order request
pub const EXTERNAL_ORDER_QUOTE_VOLUME: &str = "external_order_quote_volume";

/// Metric describing the volume of the base asset in an external match response
pub const EXTERNAL_MATCH_BASE_VOLUME: &str = "external_match_base_volume";
/// Metric describing the volume of the quote asset in an external match
/// response
pub const EXTERNAL_MATCH_QUOTE_VOLUME: &str = "external_match_quote_volume";

/// Metric describing the volume of the base asset in an external match
pub const EXTERNAL_MATCH_SETTLED_BASE_VOLUME: &str = "external_match_settled_base_volume";
/// Metric describing the volume of the quote asset in an external match
pub const EXTERNAL_MATCH_SETTLED_QUOTE_VOLUME: &str = "external_match_settled_quote_volume";

/// Metric describing the time between the price sample time and the time of
/// settlement
pub const EXTERNAL_MATCH_SETTLEMENT_DELAY: &str = "external_match_settlement_delay";

/// Metric describing the difference in price between our quote and the source
/// quote, in basis points
pub const QUOTE_PRICE_DIFF_BPS_METRIC: &str = "quote.price_diff_bps";
/// Metric describing the difference in output value net of gas between our
/// quote and the source quote, in basis points
pub const QUOTE_OUTPUT_NET_OF_GAS_DIFF_BPS_METRIC: &str = "quote.output_net_of_gas_diff_bps";
/// Metric describing the difference in output value net of fee between our
/// quote and the source quote, in basis points
pub const QUOTE_OUTPUT_NET_OF_FEE_DIFF_BPS_METRIC: &str = "quote.output_net_of_fee_diff_bps";
/// Metric describing the difference in output value net of gas and fee between
/// our quote and the source quote, in basis points
pub const QUOTE_NET_OUTPUT_DIFF_BPS_METRIC: &str = "quote.net_output_diff_bps";

/// Metric describing the value of gas sponsorship for a given request
pub const GAS_SPONSORSHIP_VALUE: &str = "gas_sponsorship_value";

/// Metric describing the number of unsuccessful relayer requests
pub const UNSUCCESSFUL_RELAYER_REQUEST_COUNT: &str = "num_unsuccessful_relayer_requests";
/// Metric describing the number of times a quote was not found
pub const QUOTE_NOT_FOUND_COUNT: &str = "num_quotes_not_found";

// ---------------
// | METRIC TAGS |
// ---------------

/// Metric tag for the asset of a deposit/withdrawal
pub const ASSET_METRIC_TAG: &str = "asset";
/// Metric tag for the description of the API key used to make the request
pub const KEY_DESCRIPTION_METRIC_TAG: &str = "key_description";
/// Metric tag for the settlement status of an external match
pub const SETTLEMENT_STATUS_TAG: &str = "did_settle";
/// Metric tag that contains a unique identifier for tracking a single request
/// through its entire lifecycle.
pub const REQUEST_ID_METRIC_TAG: &str = "request_id";
/// Metric tag for the base asset of an external order or match
pub const BASE_ASSET_METRIC_TAG: &str = "base_asset";
/// Metric tag for the path of the request
pub const REQUEST_PATH_METRIC_TAG: &str = "request_path";
/// Metric tag for the SDK version of the request
pub const SDK_VERSION_METRIC_TAG: &str = "sdk_version";

/// Metric tag for identifying the source of a quote (our server or competitor)
pub const SOURCE_NAME_TAG: &str = "source_name";
/// Metric tag for identifying the order side (buy/sell)
pub const SIDE_TAG: &str = "side";
/// Metric tag for our quoted price
pub const OUR_PRICE_TAG: &str = "our_price";
/// Metric tag for the comparison source's price
pub const SOURCE_PRICE_TAG: &str = "source_price";
/// Metric tag for our output net of gas
pub const OUR_OUTPUT_NET_OF_GAS_TAG: &str = "our_output_net_of_gas";
/// Metric tag for the comparison source's output net of gas
pub const SOURCE_OUTPUT_NET_OF_GAS_TAG: &str = "source_output_net_of_gas";
/// Metric tag for our output net of fee
pub const OUR_OUTPUT_NET_OF_FEE_TAG: &str = "our_output_net_of_fee";
/// Metric tag for the comparison source's output net of fee
pub const SOURCE_OUTPUT_NET_OF_FEE_TAG: &str = "source_output_net_of_fee";
/// Metric tag for our output net of gas and fee
pub const OUR_NET_OUTPUT_TAG: &str = "our_net_output";
/// Metric tag for the comparison source's output net of gas and fee
pub const SOURCE_NET_OUTPUT_TAG: &str = "source_net_output";

/// Metric tag to indicate that a match had its gas costs sponsored
pub const GAS_SPONSORED_METRIC_TAG: &str = "gas_sponsored";
/// Metric tag indicating the remaining value in a gas sponsorship rate limit
/// bucket
pub const REMAINING_VALUE_TAG: &str = "remaining_value";
/// Metric tag indicating the remaining time (in seconds) in a gas sponsorship
/// rate limit bucket
pub const REMAINING_TIME_TAG: &str = "remaining_time";
/// Metric tag indicating the refund asset
pub const REFUND_ASSET_TAG: &str = "refund_asset";
/// Metric tag indicating the refund amount (in whole units)
pub const REFUND_AMOUNT_TAG: &str = "refund_amount";
/// Metric tag indicating the cost per byte of L1 calldata (in L2 wei)
pub const L1_COST_PER_BYTE_TAG: &str = "l1_cost_per_byte";
/// Metric tag indicating the base fee (in wei) of the L2 portion of a
/// transaction
pub const L2_BASE_FEE_TAG: &str = "l2_base_fee";
/// Metric tag indicating whether or not the bundle was settled as part of a CoW
/// Protocol auction
pub const SETTLED_VIA_COWSWAP_TAG: &str = "settled_via_cowswap";
