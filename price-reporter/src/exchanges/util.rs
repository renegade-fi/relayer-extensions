//! Exchange connection utilities

use itertools::Itertools;
use renegade_common::types::{exchange::Exchange, token::Token};

use crate::exchanges::{
    binance::BinanceConnection, coinbase::CoinbaseConnection, connection::ExchangeConnection,
    error::ExchangeConnectionError, kraken::KrakenConnection, okx::OkxConnection,
};

/// Check if the given exchange supports the given pair
pub async fn supports_pair(
    exchange: &Exchange,
    base_token: &Token,
    quote_token: &Token,
) -> Result<bool, ExchangeConnectionError> {
    Ok(match exchange {
        Exchange::Binance => BinanceConnection::supports_pair(base_token, quote_token).await?,
        Exchange::Coinbase => CoinbaseConnection::supports_pair(base_token, quote_token).await?,
        Exchange::Kraken => KrakenConnection::supports_pair(base_token, quote_token).await?,
        Exchange::Okx => OkxConnection::supports_pair(base_token, quote_token).await?,
        Exchange::Renegade => BinanceConnection::supports_pair(base_token, quote_token).await?,
        _ => return Err(ExchangeConnectionError::unsupported_exchange(*exchange)),
    })
}

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
