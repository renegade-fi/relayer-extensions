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

/// Consolidated metric for quote comparison analysis with all data as tags
pub const QUOTE_PRICE_DIFF_BPS_METRIC: &str = "quote.price_diff_bps";

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
/// Metric tag to indicate data was recorded post decimal correction fix
pub const DECIMAL_CORRECTION_FIXED_METRIC_TAG: &str = "post_decimal_fix";

/// Metric tag for identifying the source of a quote (our server or competitor)
pub const SOURCE_NAME_TAG: &str = "source_name";

/// Metric tag for identifying the order side (buy/sell)
pub const SIDE_TAG: &str = "side";

/// Metric tag for our quoted price
pub const OUR_PRICE_TAG: &str = "our_price";

/// Metric tag for the comparison source's price
pub const SOURCE_PRICE_TAG: &str = "source_price";
