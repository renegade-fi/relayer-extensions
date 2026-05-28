//! Chain specific gas oracle contract methods
use alloy_primitives::U256;

#[cfg(feature = "arbitrum")]
mod arbitrum;
#[cfg(feature = "base")]
mod base;

#[cfg(feature = "arbitrum")]
pub use arbitrum::estimate_l1_gas_component;
#[cfg(feature = "base")]
pub use base::estimate_l1_gas_component;

#[cfg(feature = "ethereum")]
mod ethereum;
#[cfg(feature = "ethereum")]
pub use ethereum::estimate_l1_gas_component;

/// Fallback when no chain feature is enabled at compile time. Production
/// builds must select a chain via `--features arbitrum|base|ethereum` (see
/// `renegade-deploy-config.toml`). The fallback exists so workspace-level
/// `cargo check` / `cargo clippy` / IDE analysis succeed without picking a
/// chain. Calling it would only happen in a misconfigured build, so it
/// returns a clear error.
///
/// `async` is preserved so the call-site `.await` typechecks across all
/// feature flag combinations; `#[allow(clippy::unused_async)]` is needed
/// because this default-feature variant has no `.await` of its own.
#[cfg(not(any(feature = "arbitrum", feature = "base", feature = "ethereum")))]
#[allow(clippy::unused_async)]
pub async fn estimate_l1_gas_component(
    _provider: alloy::providers::DynProvider,
    _to: alloy::primitives::Address,
    _data: Vec<u8>,
) -> Result<GasPriceEstimation, String> {
    Err("auth-server built without a chain feature; enable one of `arbitrum`, \
         `base`, or `ethereum`"
        .to_string())
}

/// Result of the gas price estimation
pub struct GasPriceEstimation {
    /// The L1 gas estimate in L2 gas units
    pub gas_estimate_for_l1: U256,
    /// The L2 base fee in wei
    pub l2_base_fee: U256,
    /// The L1 base fee estimate (per byte) in wei
    pub l1_data_fee: U256,
}
