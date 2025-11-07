//! Okx API type definitions

#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use serde::Deserialize;

#[derive(Deserialize)]
pub struct OkxLiquiditySourcesResponse {
    pub code: String,
    pub data: Vec<OkxLiquiditySource>,
}

#[derive(Deserialize)]
pub struct OkxLiquiditySource {
    pub id: String,
    pub logo: String,
    pub name: String,
}
