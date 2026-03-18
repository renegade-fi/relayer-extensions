//! Contract interaction helpers

#[cfg(all(feature = "arbitrum", not(feature = "base")))]
mod arbitrum;
#[cfg(feature = "base")]
mod base;
