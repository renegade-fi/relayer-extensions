//! Exchange connection utilities

use itertools::Itertools;
use renegade_types_core::{Exchange, Token};

use crate::exchanges::error::ExchangeConnectionError;

/// Returns whether or not the given exchange lists both the tokens in the pair
/// separately
pub fn exchange_lists_pair_tokens(
    exchange: Exchange,
    base_token: &Token,
    quote_token: &Token,
) -> bool {
    let listing_exchanges = get_listing_exchanges(base_token, quote_token);
    listing_exchanges.contains(&exchange)
}

/// Returns the list of exchanges that list both the base and quote tokens.
///
/// Note: This does not mean that each exchange has a market for the pair,
/// just that it separately lists both tokens.
pub fn get_listing_exchanges(base_token: &Token, quote_token: &Token) -> Vec<Exchange> {
    // Compute the intersection of the supported exchanges for each of the assets
    // in the pair
    let base_token_supported_exchanges = base_token.supported_exchanges();
    let quote_token_supported_exchanges = quote_token.supported_exchanges();
    base_token_supported_exchanges
        .intersection(&quote_token_supported_exchanges)
        .copied()
        .collect_vec()
}

/// Get the exchange ticker for the base token in the given pair
pub fn get_base_exchange_ticker(
    base_token: Token,
    quote_token: Token,
    exchange: Exchange,
) -> Result<String, ExchangeConnectionError> {
    base_token.get_exchange_ticker(exchange).ok_or(ExchangeConnectionError::UnsupportedPair(
        base_token,
        quote_token,
        exchange,
    ))
}

/// Get the exchange ticker for the quote token in the given pair
pub fn get_quote_exchange_ticker(
    base_token: Token,
    quote_token: Token,
    exchange: Exchange,
) -> Result<String, ExchangeConnectionError> {
    quote_token.get_exchange_ticker(exchange).ok_or(ExchangeConnectionError::UnsupportedPair(
        base_token,
        quote_token,
        exchange,
    ))
}

/// Compute `(best_bid + best_offer) / 2`, rejecting non-finite or non-positive
/// inputs. Returns `None` so the caller can emit no midpoint rather than a
/// corrupted value.
///
/// Protects against `bid=0, offer=N` style inputs that would otherwise yield
/// `N/2` (the failure mode behind the 2026-05-08 cbBTC pricing incident),
/// plus NaN/Inf inputs that would corrupt downstream consumers.
pub fn safe_midpoint(best_bid: f64, best_offer: f64) -> Option<f64> {
    if !best_bid.is_finite() || best_bid <= 0.0 {
        return None;
    }
    if !best_offer.is_finite() || best_offer <= 0.0 {
        return None;
    }
    Some((best_bid + best_offer) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::safe_midpoint;

    #[test]
    fn normal_inputs_return_midpoint() {
        assert_eq!(safe_midpoint(100.0, 102.0), Some(101.0));
    }

    #[test]
    fn zero_bid_returns_none() {
        // Reproduces the 2026-05-08 cbBTC failure mode: a zero on either side
        // would otherwise produce `best_offer / 2` ≈ half of the real price.
        assert_eq!(safe_midpoint(0.0, 79_675.0), None);
    }

    #[test]
    fn zero_offer_returns_none() {
        assert_eq!(safe_midpoint(79_674.0, 0.0), None);
    }

    #[test]
    fn negative_inputs_return_none() {
        assert_eq!(safe_midpoint(-1.0, 100.0), None);
        assert_eq!(safe_midpoint(100.0, -1.0), None);
    }

    #[test]
    fn non_finite_inputs_return_none() {
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(safe_midpoint(bad, 100.0), None, "bid {bad}");
            assert_eq!(safe_midpoint(100.0, bad), None, "offer {bad}");
        }
    }
}
