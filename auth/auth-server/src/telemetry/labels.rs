//! Metric names and tags

// ----------------
// | METRIC NAMES |
// ----------------

/// Metric describing the number of atomic matches requested
pub const NUM_ATOMIC_MATCH_REQUESTS: &str = "num_atomic_match_requests";

/// Metric describing the volume of requested external orders
pub const EXTERNAL_ORDER_VOLUME: &str = "external_order_volume";

/// Metric describing the volume of atomic match bundles
pub const EXTERNAL_MATCH_BUNDLE_VOLUME: &str = "external_match_bundle_volume";

// ---------------
// | METRIC TAGS |
// ---------------

/// Metric tag for the asset of a deposit/withdrawal
pub const ASSET_METRIC_TAG: &str = "asset";
/// Metric tag for the description of the API key used to make the request
pub const KEY_DESCRIPTION_METRIC_TAG: &str = "key_description";
