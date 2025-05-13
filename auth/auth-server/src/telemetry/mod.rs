//! Defines helpers for recording metrics
pub mod helpers;
pub mod labels;
pub mod quote_comparison;
pub mod sources;

/// The threshold beyond which to ignore a quote's fill ratio
///
/// We ignore quotes beyond this value as they're likely to be spam, or are far
/// beyond expected external match liquidity so as to be useless for telemetry.
///
/// Specified in USDC
pub const QUOTE_FILL_RATIO_IGNORE_THRESHOLD: u128 = 100_000 * 10u128.pow(6u32); // $100,000 of USDC
