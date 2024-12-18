//! Metric names and tags

// ----------------
// | METRIC NAMES |
// ----------------

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
pub const DECIMAL_CORRECTION_FIXED_METRIC_TAG: &str = "post_decimal_correction_fix";
