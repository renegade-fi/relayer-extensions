//! Constants for metric labels and metric names

/// Metric for the net execution cost of the swap in USD
pub const SWAP_EXECUTION_COST_METRIC_NAME: &str = "swap_execution_cost";

/// Metric for the gas cost of execution in USD
pub const SWAP_GAS_COST_METRIC_NAME: &str = "swap_gas_cost";

/// Metric for the notional volume of the swap in USD
pub const SWAP_NOTIONAL_VOLUME_METRIC_NAME: &str = "swap_notional_volume";

/// Metric for the relative spread between execution price and Binance price
pub const SWAP_RELATIVE_SPREAD_METRIC_NAME: &str = "swap_relative_spread";

/// Metric describing the price deviation of a quote from the Renegade price
pub const QUOTE_PRICE_DEVIATION: &str = "quote_price_deviation";

/// Metric tag for the asset's ticker symbol or address
pub const ASSET_TAG: &str = "asset";

/// Metric tag for the trade side, either `"buy"` or `"sell"`
pub const TRADE_SIDE_FACTOR_TAG: &str = "side";

/// Metric tag for the (environment-agnostic) chain name
pub const CHAIN_TAG: &str = "chain";

/// Metric tag for the venue that executed a swap
pub const VENUE_TAG: &str = "venue";
