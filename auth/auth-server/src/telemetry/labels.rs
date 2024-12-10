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

// ---------------
// | METRIC TAGS |
// ---------------

/// Metric tag for the asset of a deposit/withdrawal
pub const ASSET_METRIC_TAG: &str = "asset";
/// Metric tag for the description of the API key used to make the request
pub const KEY_DESCRIPTION_METRIC_TAG: &str = "key_description";
