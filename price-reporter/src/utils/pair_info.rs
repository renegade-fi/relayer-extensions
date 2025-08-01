//! Types and utilities for PairInfo
//!
//! PairInfo is the ticker-based key we use to de-duplicate price streams. In
//! contrast, `PriceTopic` is a tuple of (Exchange, Token, Token) that uses
//! addresses for uniqueness. This is necessary in a multi-chain environment
//! where multiple addresses can map to the same ticker.
use std::str::FromStr;

use derivative::Derivative;
use renegade_common::types::{
    chain::Chain,
    exchange::Exchange,
    token::{default_chain, Token, USDC_TICKER},
};
use renegade_util::err_str;

use crate::{
    errors::ServerError,
    exchanges::{
        binance::BinanceConnection, coinbase::CoinbaseConnection, connection::ExchangeConnection,
        kraken::KrakenConnection, okx::OkxConnection,
    },
    utils::{
        canonical_exchange::get_canonical_exchange, default_exchange_stable, get_token_and_chain,
        resolve_tokens_and_chain, PriceTopic,
    },
};

/// Used to uniquely identify a price stream
#[derive(Derivative, Clone, Debug)]
#[derivative(PartialEq, Eq, Hash)]
pub struct PairInfo {
    /// The exchange
    pub exchange: Exchange,
    /// The base ticker
    pub base: String,
    /// The quote ticker
    pub quote: String,
    /// The chain
    #[derivative(PartialEq = "ignore", Hash = "ignore")]
    pub chain: Chain,
}

impl PairInfo {
    /// Create a new pair info
    pub fn new(exchange: Exchange, base: String, quote: String, chain: Option<Chain>) -> Self {
        Self { exchange, base, quote, chain: chain.unwrap_or(default_chain()) }
    }

    /// Create a new pair info from two tokens
    pub fn new_from_tokens(
        exchange: Exchange,
        base: &Token,
        quote: &Token,
    ) -> Result<Self, ServerError> {
        assert_eq!(base.get_chain(), quote.get_chain());
        let base_ticker = base.get_ticker().ok_or_else(|| {
            ServerError::InvalidPairInfo(format!("unable to get ticker for {}", base))
        })?;
        let quote_ticker = quote.get_ticker().ok_or_else(|| {
            ServerError::InvalidPairInfo(format!("unable to get ticker for {}", quote))
        })?;

        Ok(Self { exchange, base: base_ticker, quote: quote_ticker, chain: quote.get_chain() })
    }

    /// Create a new pair info with the default stable token of the given
    /// exchange
    pub fn new_default_stable(exchange: Exchange, base_mint: &str) -> Result<Self, ServerError> {
        let (base, chain) = get_token_and_chain(base_mint).ok_or_else(|| {
            ServerError::InvalidPairInfo(format!("invalid token `{}`", base_mint))
        })?;
        let quote = default_exchange_stable(&exchange, chain);

        Self::new_from_tokens(exchange, &base, &quote)
    }

    /// Derive a new pair info using the canonical exchange and its default
    /// stable
    pub fn new_canonical_exchange(base_mint: &str) -> Result<Self, ServerError> {
        let canonical_exchange = get_canonical_exchange(base_mint)?;

        Self::new_default_stable(canonical_exchange, base_mint)
    }

    /// Get the base token for a given pair info
    pub fn base_token(&self) -> Token {
        Token::from_ticker_on_chain(self.base.as_str(), self.chain)
    }

    /// Get the quote token for a given pair info
    pub fn quote_token(&self) -> Token {
        Token::from_ticker_on_chain(self.quote.as_str(), self.chain)
    }

    // ---------------
    // | Price Topic |
    // ---------------

    /// Parse the pair info from a given topic
    pub fn from_topic(topic: &str) -> Result<Self, ServerError> {
        let parts: Vec<&str> = topic.split('-').collect();
        let exchange =
            Exchange::from_str(parts[0]).map_err(err_str!(ServerError::InvalidPairInfo))?;
        let base_mint = parts[1];
        // Renegade topics may omit the quote: use default stable (USDC) if so
        if exchange == Exchange::Renegade && parts.len() == 2 {
            return Self::new_default_stable(exchange, base_mint);
        }
        // Otherwise, expect a quote mint
        let quote_mint = parts[2];

        // Resolve both mints on the same chain
        if let Some((base, quote, _chain)) = resolve_tokens_and_chain(base_mint, quote_mint) {
            // For Renegade, enforce that the quote is USDC
            if exchange == Exchange::Renegade {
                Self::enforce_usdc(&quote)?;
            }
            return Self::new_from_tokens(exchange, &base, &quote);
        }

        Err(ServerError::InvalidPairInfo(format!(
            "invalid token pair `{}`-`{}`",
            base_mint, quote_mint
        )))
    }

    /// Ensure the given Token's ticker is USDC
    fn enforce_usdc(token: &Token) -> Result<(), ServerError> {
        let ticker = token.get_ticker().ok_or_else(|| {
            ServerError::InvalidPairInfo(format!("unable to get ticker for {}", token))
        })?;
        if ticker != USDC_TICKER {
            return Err(ServerError::InvalidPairInfo(format!("expected USDC, got {}", ticker)));
        }
        Ok(())
    }

    /// Get the topic name for a given pair info as a string
    pub fn to_topic(&self) -> String {
        format!("{}-{}-{}", self.exchange, self.base, self.quote)
    }

    // --------------
    // | Validation |
    // --------------

    /// Validate a pair info tuple, checking that the exchange supports the base
    /// and quote tokens
    pub async fn validate_subscription(&self) -> Result<(), ServerError> {
        let (exchange, base, quote) = (self.exchange, self.base_token(), self.quote_token());
        if exchange == Exchange::UniswapV3 {
            return Err(ServerError::InvalidPairInfo("UniswapV3 is not supported".to_string()));
        }

        if !self.is_supported().await? {
            return Err(ServerError::InvalidPairInfo(format!(
                "{} does not support the pair ({}, {})",
                self.exchange, base, quote
            )));
        }

        Ok(())
    }

    /// Check if the given exchange supports the given pair
    pub async fn is_supported(&self) -> Result<bool, ServerError> {
        // If the pair is a unit pair (e.g. USDT-USDT), we don't need to check
        // if the exchange supports it
        if self.is_unit_pair() {
            return Ok(true);
        }

        let (exchange, base_token, quote_token) =
            (self.exchange, self.base_token(), self.quote_token());

        Ok(match exchange {
            Exchange::Binance => {
                BinanceConnection::supports_pair(&base_token, &quote_token).await?
            },
            Exchange::Coinbase => {
                CoinbaseConnection::supports_pair(&base_token, &quote_token).await?
            },
            Exchange::Kraken => KrakenConnection::supports_pair(&base_token, &quote_token).await?,
            Exchange::Okx => OkxConnection::supports_pair(&base_token, &quote_token).await?,
            Exchange::Renegade => {
                BinanceConnection::supports_pair(&base_token, &quote_token).await?
            },
            _ => return Err(ServerError::invalid_pair_info(self)),
        })
    }

    /// Returns whether the pair is a unit pair
    pub fn is_unit_pair(&self) -> bool {
        self.base == self.quote
    }

    // --------------------
    // | Quote Conversion |
    // --------------------

    /// Whether the pair info requires quote conversion
    pub fn requires_quote_conversion(&self) -> Result<bool, ServerError> {
        // We only convert if explicitly using Renegade topic
        let is_renegade = self.exchange == Exchange::Renegade;

        let base_mint = self.base_token().get_addr();
        let canonical_exchange = get_canonical_exchange(&base_mint)?;

        // If canonical exchange is Coinbase, we don't need to convert (USD == USDC)
        let is_canonical_coinbase = canonical_exchange == Exchange::Coinbase;

        // If the canonical exchange's default stable is USDC, we don't need to convert
        let is_canonical_stable_usdc =
            default_exchange_stable(&canonical_exchange, self.chain).get_ticker().unwrap()
                == USDC_TICKER;

        Ok(is_renegade && !is_canonical_coinbase && !is_canonical_stable_usdc)
    }

    /// Get the pair info for the quote conversion pair
    ///
    /// This is just the price of USDC against the default stable of the
    /// exchange.
    pub fn get_conversion_pair(&self) -> Result<PairInfo, ServerError> {
        let usdc = Token::from_ticker_on_chain(USDC_TICKER, self.chain);
        Self::new_default_stable(self.exchange, usdc.get_addr().as_str())
    }
}

impl From<PairInfo> for PriceTopic {
    fn from(pair_info: PairInfo) -> Self {
        (pair_info.exchange, pair_info.base_token(), pair_info.quote_token())
    }
}
