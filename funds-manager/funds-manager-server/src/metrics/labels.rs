//! Constants for metric labels and metric names

/// Metric for the net execution cost of the swap in USD
pub const SWAP_EXECUTION_COST_METRIC_NAME: &str = "swap_execution_cost";

/// Metric for the gas cost of execution in USD
pub const SWAP_GAS_COST_METRIC_NAME: &str = "swap_gas_cost";

/// Metric for the notional volume of the swap in USD
pub const SWAP_NOTIONAL_VOLUME_METRIC_NAME: &str = "swap_notional_volume";

/// Metric for the relative spread between execution price and Binance price
pub const SWAP_RELATIVE_SPREAD_METRIC_NAME: &str = "swap_relative_spread";

/// Metric tag for the asset's ticker symbol or address
pub const ASSET_TAG: &str = "asset";

/// Metric tag for the trade side, either `"buy"` or `"sell"`
pub const TRADE_SIDE_FACTOR_TAG: &str = "side";

/// Metric tag for the transaction hash of the swap
pub const HASH_TAG: &str = "hash";
