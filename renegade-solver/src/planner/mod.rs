//! Defines helpers to compute values used during the transaction placement.

use crate::tx_store::store::{L2Position, TxTiming};

/// The measurements used to compute the transaction placement.
#[derive(Clone, Debug)]
pub struct Measurements {
    /// Measured from the time the transaction is sent to the network to the
    /// time it is observed in a block.
    pub send_to_inclusion_ms: u64,
    /// The duration of a single flashblock window in milliseconds.
    pub flashblock_duration_ms: u64,
    /// The duration of a full L2 block in milliseconds.
    pub block_duration_ms: u64,
    /// The number of flashblocks the builder is ahead of the WS snapshot.
    pub ws_lag_flashblocks: u64,
}

/// The default value for the `send_to_inclusion_ms` field.
const DEFAULT_SEND_TO_INCLUSION_MS: u64 = 400;
/// The default value for the `flashblock_duration_ms` field.
const DEFAULT_FLASHBLOCK_DURATION_MS: u64 = 200;
/// The default value for the `block_duration_ms` field.
const DEFAULT_BLOCK_DURATION_MS: u64 = 2000;
/// The default value for the `ws_lag_flashblocks` field.
const DEFAULT_WS_LAG_FLASHBLOCKS: u64 = 2;

impl Default for Measurements {
    fn default() -> Self {
        Self {
            send_to_inclusion_ms: DEFAULT_SEND_TO_INCLUSION_MS,
            flashblock_duration_ms: DEFAULT_FLASHBLOCK_DURATION_MS,
            block_duration_ms: DEFAULT_BLOCK_DURATION_MS,
            ws_lag_flashblocks: DEFAULT_WS_LAG_FLASHBLOCKS,
        }
    }
}

impl Measurements {
    /// The number of flashblocks per L2 block.
    pub fn flashblocks_per_block(&self) -> u64 {
        self.block_duration_ms.div_ceil(self.flashblock_duration_ms).max(1)
    }

    /// The latency expressed as a number of windows, where a window is defined
    /// as the time period marked by the observation of 2 consecutive
    /// flashblocks.
    pub fn latency_windows(&self) -> u64 {
        self.send_to_inclusion_ms.div_ceil(self.flashblock_duration_ms)
    }
}

/// The send plan for a given target position and measurements.
#[derive(Clone, Debug)]
pub struct SendPlan {
    /// The target position.
    #[allow(dead_code)]
    pub target: L2Position,
    /// The trigger position.
    pub trigger: L2Position,
    /// The milliseconds to wait after the trigger flashblock is observed before
    /// broadcasting the transaction.
    pub buffer_ms: u64,
}

/// Computes the send plan for a given target position and measurements.
pub fn compute_send_plan(target: L2Position, m: &Measurements) -> SendPlan {
    let delta = m.latency_windows() + m.ws_lag_flashblocks;
    let trigger = target.sub_flashblocks(delta, m.flashblocks_per_block());

    let latency_ms = m.latency_windows() * m.flashblock_duration_ms;
    let buffer_ms = latency_ms.saturating_sub(m.send_to_inclusion_ms);

    SendPlan { target, trigger, buffer_ms }
}

impl From<SendPlan> for TxTiming {
    fn from(p: SendPlan) -> Self {
        TxTiming { trigger: p.trigger, buffer_ms: p.buffer_ms }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_send_plan_default_target_flashblock() {
        // Default measurements: window_ms=200, block_ms=2000 => windows_per_block=10
        // send_to_inclusion_ms=400 => latency_windows=2, ws_lead_windows=2
        // Default measurements:
        // - 10 flashblocks per block
        // - active flashblock being built leads websocket event by 2 flashblocks
        // - latency of sending to inclusion is 400ms or 2 windows
        // therefore we must trigger on 100#1 - 4 flashblocks = 99#7
        let measurements = Measurements::default();

        let target = L2Position { l2_block: 100, flashblock: 1 };
        let plan = compute_send_plan(target, &measurements);

        assert_eq!(plan.trigger.l2_block, 99);
        assert_eq!(plan.trigger.flashblock, 7);
        assert_eq!(plan.buffer_ms, 0);
    }
}
