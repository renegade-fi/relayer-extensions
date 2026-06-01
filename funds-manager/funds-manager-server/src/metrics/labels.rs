//! Constants for metric labels and metric names

/// Metric for the net execution cost of the swap in USD
pub const SWAP_EXECUTION_COST_METRIC_NAME: &str = "swap_execution_cost";

/// Metric for the net execution cost of swaps whose recorded spread exceeded
/// `OUTLIER_RELATIVE_SPREAD_THRESHOLD`. Routed here instead of
/// `SWAP_EXECUTION_COST_METRIC_NAME` so dashboards (notably the
/// `cumsum`-based Cumulative Swap Execution Costs widget) are not
/// corrupted by reference-price artifacts. See cost.rs for the routing
/// decision.
pub const SWAP_EXECUTION_COST_ARTIFACT_METRIC_NAME: &str = "swap_execution_cost_artifact";

/// Metric for the gas cost of execution in USD
pub const SWAP_GAS_COST_METRIC_NAME: &str = "swap_gas_cost";

/// Gauge: number of inbound Fireblocks webhooks currently being verified +
/// dispatched off the request path. A rising value is the saturation signal —
/// the in-flight backlog that, when ACK latency rode the shared runtime, drove
/// the Fireblocks webhook retry storm.
pub const FIREBLOCKS_WEBHOOK_INFLIGHT_METRIC_NAME: &str = "fireblocks_webhook_inflight";

/// Histogram (milliseconds): time from receiving a Fireblocks webhook to
/// finishing its verification + dispatch. ACK latency is now decoupled from
/// this, so a rise here surfaces runtime saturation before it can delay ACKs.
pub const FIREBLOCKS_WEBHOOK_PROCESS_LATENCY_MS_METRIC_NAME: &str =
    "fireblocks_webhook_process_latency_ms";

/// Metric for the notional volume of the swap in USD
pub const SWAP_NOTIONAL_VOLUME_METRIC_NAME: &str = "swap_notional_volume";

/// Metric for the relative spread between execution price and reference price
pub const SWAP_RELATIVE_SPREAD_METRIC_NAME: &str = "swap_relative_spread";

/// Metric describing the price deviation of a quote from the Renegade price
pub const QUOTE_PRICE_DEVIATION: &str = "quote_price_deviation";

/// Metric for the USDC volume transferred through the darkpool in the swap
pub const SELF_TRADE_VOLUME_USDC_METRIC_NAME: &str = "self_trade_volume";

/// Metric tag for the asset's ticker symbol or address
pub const ASSET_TAG: &str = "asset";

/// Metric tag for the trade side, either `"buy"` or `"sell"`
pub const TRADE_SIDE_FACTOR_TAG: &str = "side";

/// Metric tag for the (environment-agnostic) chain name
pub const CHAIN_TAG: &str = "chain";

/// Metric tag for the venue that executed a swap
pub const VENUE_TAG: &str = "venue";
