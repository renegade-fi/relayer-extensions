//! Gas estimation for external matches

use alloy::primitives::U256;

use crate::server::Server;

pub mod constants;
pub mod gas_cost_sampler;
pub mod gas_oracles;

// ---------------
// | Server Impl |
// ---------------

impl Server {
    /// Get the latest estimate of the gas cost for an external match
    pub async fn get_gas_cost_estimate(&self) -> U256 {
        self.gas_cost_sampler.get_latest_estimate().await
    }
}
