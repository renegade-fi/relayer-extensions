//! API connectors for the auth server
//!
//! These connectors are used to connect the auth server's API to various
//! expecter APIs.

// `okx_market_maker` still imports v1-only external-api types
// (`GET_DEPTH_FOR_ALL_PAIRS_ROUTE`, `renegade_common::types::token`) and is
// gated off until it is migrated to v2 (`GET_MARKETS_DEPTH_ROUTE`,
// `renegade_types_core::Token`). Tracked separately from the RFQT v2 port.
// pub mod okx_market_maker;
pub mod rfqt;
