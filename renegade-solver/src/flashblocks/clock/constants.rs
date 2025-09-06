//! Constants for the FlashblockClock.

/// The frequency of flashblocks according to the [docs](https://docs.base.org/base-chain/network-information/block-building).
pub const DEFAULT_FLASHBLOCK_MS: f64 = 200.0;
/// The frequency of L2 blocks according to the [docs](https://docs.base.org/base-chain/network-information/block-building).
pub const DEFAULT_L2_MS: f64 = 2_000.0;
/// The initial seed value for the websocket drift EMA.
pub const DEFAULT_FORECAST_ERROR_MS: f64 = 200.0;
/// EMA window lengths for flashblock durations.
pub const FLASHBLOCK_WINDOW: u32 = 10;
/// EMA window lengths for L2 block durations.
pub const L2_WINDOW: u32 = 2;
/// EMA window lengths for websocket drift.
pub const FORECAST_ERROR_WINDOW: u32 = 4;
/// This lead time accounts for the fact that we observe the completion of the
/// building phase of a flashblock, not the start over the Websocket.
/// To target a specific flashblock, we must ensure our transaction is in the
/// builder's mempool snapshot for that flashblock, so we lead by approximately
/// one flashblock duration.
pub const BLOCK_BUILDER_DEADLINE_LEAD_TIME_MS: u64 = 200;
/// This lead time accounts for the following facts:
/// - flashblock #0 is special in that it does not contain user txns, so we can
///   be in the mempool without being included in that flashblock.
/// - if a transaction has a miner tip higher than the minimum miner tip already
///   in the block, it will not be included in the block. Therefore, we can
///   arrive sometime during the building phase of (N-1)#10 and still be
///   included in N#1, where N is the target block number and 1 is the
///   flashblock number.
pub const FIRST_FLASHBLOCK_LEAD_TIME_MS: u64 = 300;
