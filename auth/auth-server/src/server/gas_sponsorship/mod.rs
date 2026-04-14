//! Handler code for proxied relayer requests
//!
//! At a high level the server must first authenticate the request, then forward
//! it to the relayer with admin authentication

use alloy_primitives::U256;
use auth_server_api::{
    GasSponsorshipInfo, GasSponsorshipQueryParams, SponsoredMatchResponse, SponsoredQuoteResponse,
};
use price_reporter_client::error::PriceReporterClientError;

use refund_calculation::{apply_gas_sponsorship_to_match_bundle, apply_gas_sponsorship_to_quote};
use renegade_circuit_types::fixed_point::FixedPoint;
use renegade_external_api::http::external_match::{ExternalMatchResponse, ExternalQuoteResponse};
use renegade_external_api::types::{ApiTimestampedPriceFp, ExternalOrder};
use renegade_types_core::Token;
use serde::{Deserialize, Serialize};

use super::Server;
use crate::error::AuthServerError;
use crate::server::helpers::generate_quote_uuid;

pub mod contract_interaction;
pub mod refund_calculation;

// -------------
// | Constants |
// -------------

/// The ticker for WETH
const WETH_TICKER: &str = "WETH";

// ---------
// | Types |
// ---------

/// Internal struct for caching gas sponsorship info along with
/// the original price from the relayer's signed quote.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CachedSponsorshipInfo {
    /// The gas sponsorship info to return to clients
    pub gas_sponsorship_info: GasSponsorshipInfo,
    /// The original price from the relayer's signed quote,
    /// needed to restore the quote for signature verification
    #[serde(with = "renegade_external_api::serde_helpers::f64_as_string")]
    pub original_price: f64,
    /// The original fixed-point price from the quote's match result,
    /// needed to restore the quote for signature verification
    pub original_price_fp: ApiTimestampedPriceFp,
}

// ---------------
// | Server Impl |
// ---------------

/// Handle a proxied request
impl Server {
    /// Generate gas sponsorship info for a given user's order, if permissible
    /// according to the rate limit and query params
    pub(crate) async fn generate_sponsorship_info(
        &self,
        key_desc: &str,
        order: &ExternalOrder,
        query_params: &GasSponsorshipQueryParams,
    ) -> Result<GasSponsorshipInfo, AuthServerError> {
        // Parse query params
        let (sponsorship_disabled, refund_address, refund_native_eth) =
            query_params.get_or_default();

        // Check gas sponsorship rate limit
        let rate_limited = !self.check_gas_sponsorship_rate_limit(key_desc).await?;

        let expected_quote_amount =
            match self.get_quote_amount(order, FixedPoint::zero() /* relayer_fee */).await {
                Ok(amt) => amt,
                Err(AuthServerError::PriceReporter(PriceReporterClientError::StreamMissing(_))) => {
                    return Ok(GasSponsorshipInfo::zero());
                },
                Err(e) => return Err(e),
            };

        let expected_quote_amount_f64 = Token::usdc().convert_to_decimal(expected_quote_amount);
        let order_too_small = expected_quote_amount_f64 < self.min_sponsored_order_quote_amount;

        let sponsor_match = !(rate_limited || sponsorship_disabled || order_too_small);
        if !sponsor_match {
            return Ok(GasSponsorshipInfo::zero());
        }

        let refund_amount = self.compute_refund_amount_for_order(order, refund_native_eth).await?;
        GasSponsorshipInfo::new(refund_amount, refund_native_eth, refund_address)
            .map_err(AuthServerError::gas_sponsorship)
    }

    /// Construct a sponsored match response from an external match response
    pub(crate) fn construct_sponsored_match_response(
        &self,
        mut external_match_resp: ExternalMatchResponse,
        gas_sponsorship_info: GasSponsorshipInfo,
        sponsorship_nonce: U256,
    ) -> Result<SponsoredMatchResponse, AuthServerError> {
        // Generate calldata for the gas sponsorship contract
        let gas_sponsor_calldata = self.generate_gas_sponsor_calldata(
            &external_match_resp,
            &gas_sponsorship_info,
            sponsorship_nonce,
        )?;

        let mut tx = external_match_resp.match_bundle.settlement_tx;
        tx = tx.to(self.gas_sponsor_address);
        tx.input.input = Some(gas_sponsor_calldata);
        external_match_resp.match_bundle.settlement_tx = tx;

        // The `ExternalMatchResponse` from the relayer doesn't account for gas
        // sponsorship, so we need to update the match bundle to reflect the
        // refund.
        if gas_sponsorship_info.requires_match_result_update() {
            apply_gas_sponsorship_to_match_bundle(
                &mut external_match_resp.match_bundle,
                gas_sponsorship_info.refund_amount,
            )?;
        }

        Ok(SponsoredMatchResponse {
            match_bundle: external_match_resp.match_bundle,
            gas_sponsorship_info: Some(gas_sponsorship_info),
        })
    }

    /// Construct a sponsored quote response from an external quote response
    ///
    /// Returns a tuple of (SponsoredQuoteResponse,
    /// Option<CachedSponsorshipInfo>).
    ///
    /// The `CachedSponsorshipInfo` contains the original price and should be
    /// cached in Redis to restore the quote during assembly.
    pub(crate) fn construct_sponsored_quote_response(
        &self,
        mut external_quote_response: ExternalQuoteResponse,
        gas_sponsorship_info: GasSponsorshipInfo,
    ) -> Result<(SponsoredQuoteResponse, Option<CachedSponsorshipInfo>), AuthServerError> {
        let quote = &mut external_quote_response.signed_quote.quote;

        // Update quote price / receive amount to reflect sponsorship
        // and capture the original price for caching
        let cached_info = if gas_sponsorship_info.requires_match_result_update() {
            let original_price_fp = quote.match_result.price_fp.clone();
            let original_price = apply_gas_sponsorship_to_quote(quote, &gas_sponsorship_info)?;
            Some(CachedSponsorshipInfo {
                gas_sponsorship_info: gas_sponsorship_info.clone(),
                original_price,
                original_price_fp,
            })
        } else {
            None
        };

        let response = SponsoredQuoteResponse {
            signed_quote: external_quote_response.signed_quote,
            gas_sponsorship_info: Some(gas_sponsorship_info),
        };

        Ok((response, cached_info))
    }

    /// Cache the gas sponsorship info for a given quote in Redis
    pub async fn cache_quote_gas_sponsorship_info(
        &self,
        res: &SponsoredQuoteResponse,
        cached_info: CachedSponsorshipInfo,
    ) -> Result<(), AuthServerError> {
        let redis_key = generate_quote_uuid(&res.signed_quote);
        self.write_sponsorship_info_to_redis(redis_key, &cached_info).await
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    /// Simulates the UNFIXED CachedSponsorshipInfo — bare f64, serialized
    /// as a JSON number (e.g. `1234.5678`).
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct BareF64Cache {
        original_price: f64,
    }

    /// Simulates the FIXED CachedSponsorshipInfo — f64 serialized as a
    /// JSON string (e.g. `"1234.5678"`), matching ApiTimestampedPrice.
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct StringF64Cache {
        #[serde(with = "renegade_external_api::serde_helpers::f64_as_string")]
        original_price: f64,
    }

    /// Proof of concept: demonstrate that the JSON format of the bare f64
    /// (number) differs from the f64_as_string format (string). The HMAC
    /// is computed over the quote's JSON bytes, where the price field uses
    /// f64_as_string. If the cached price round-trips through a different
    /// JSON format, it could produce a different f64 bit pattern, causing
    /// the re-serialized quote to differ → HMAC mismatch.
    #[test]
    fn test_f64_serialization_format_mismatch() {
        // A realistic price value from a quote
        let price: f64 = 0.000009483294637281045;

        // --- Bare f64 (the bug) ---
        let bare = BareF64Cache { original_price: price };
        let bare_json = serde_json::to_string(&bare).unwrap();

        // --- f64_as_string (the fix) ---
        let string = StringF64Cache { original_price: price };
        let string_json = serde_json::to_string(&string).unwrap();

        println!("Original f64 bits:    {:064b}", price.to_bits());
        println!("Original to_string(): {}", price);
        println!();
        println!("Bare f64 JSON:        {bare_json}");
        println!("f64_as_string JSON:   {string_json}");

        // The JSON formats are structurally different:
        //   bare:   {"original_price":0.000009483294637281045}
        //   string: {"original_price":"0.000009483294637281045"}
        // This is the root cause — different serialization paths for the
        // same logical value.

        // Round-trip through bare JSON (simulating Redis write/read)
        let bare_restored: BareF64Cache = serde_json::from_str(&bare_json).unwrap();
        let bare_restored_bits = bare_restored.original_price.to_bits();

        // Round-trip through string JSON
        let string_restored: StringF64Cache = serde_json::from_str(&string_json).unwrap();
        let string_restored_bits = string_restored.original_price.to_bits();

        println!();
        println!("Bare round-trip bits:   {:064b}", bare_restored_bits);
        println!("String round-trip bits: {:064b}", string_restored_bits);
        println!(
            "Bits match original:    bare={}, string={}",
            bare_restored_bits == price.to_bits(),
            string_restored_bits == price.to_bits(),
        );

        // The critical check: after restoring the price from cache and
        // re-serializing the quote (which uses f64_as_string), do we get
        // the same string the relayer signed?
        let original_display = price.to_string();
        let bare_restored_display = bare_restored.original_price.to_string();
        let string_restored_display = string_restored.original_price.to_string();

        println!();
        println!("Original Display:        {original_display}");
        println!("Bare restored Display:   {bare_restored_display}");
        println!("String restored Display: {string_restored_display}");

        // The f64_as_string path is guaranteed to round-trip correctly
        // because it uses the same serialization format (Display/to_string)
        // on both sides. The bare path uses a *different* format (JSON
        // number via ryu) which is not guaranteed to produce the same
        // Display output after round-tripping.
        assert_eq!(
            string_restored_display, original_display,
            "f64_as_string round-trip must preserve Display output exactly"
        );
    }

    /// Demonstrate that serde_json's bare number format and f64::to_string()
    /// can produce different representations for the same value, which is
    /// the mechanism by which HMAC verification can fail.
    #[test]
    fn test_bare_vs_string_serialization_difference() {
        // Try a range of realistic price values to find divergences
        let test_prices: Vec<f64> = vec![
            1.0 / 3.0,               // repeating decimal
            0.1 + 0.2,               // classic floating point
            std::f64::consts::PI,    // irrational
            1e-15,                   // very small
            1.7976931348623157e+308, // near f64::MAX
            0.000009483294637281045, // realistic crypto price
            2999.4800000000005,      // ETH-like price with rounding artifact
        ];

        println!(
            "{:<35} | {:<30} | {:<30} | match?",
            "value", "serde_json number", "f64::to_string()"
        );
        println!("{}", "-".repeat(105));

        for price in &test_prices {
            // What serde_json produces for a bare f64 (JSON number)
            let serde_repr = serde_json::to_string(price).unwrap();
            // What f64_as_string would produce (Display trait)
            let display_repr = price.to_string();

            let matches = serde_repr == display_repr;
            println!("{price:<35e} | {serde_repr:<30} | {display_repr:<30} | {matches}");
        }
    }
}
