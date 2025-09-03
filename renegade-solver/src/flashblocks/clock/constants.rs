//! Constants for the FlashblockClock.

/// The frequency of flashblocks according to the [docs](https://docs.base.org/base-chain/network-information/block-building).
pub const DEFAULT_FLASHBLOCK_MS: u64 = 200;
/// The frequency of L2 blocks according to the [docs](https://docs.base.org/base-chain/network-information/block-building).
pub const DEFAULT_L2_MS: u64 = 2_000;
/// EMA window lengths for flashblock durations.
pub const FLASHBLOCK_WINDOW: usize = 10;
/// EMA window lengths for L2 block durations.
pub const L2_WINDOW: usize = 2;
