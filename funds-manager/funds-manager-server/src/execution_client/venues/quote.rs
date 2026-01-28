//! Type definitions for execution quotes

use std::fmt::Display;

use alloy_primitives::U256;
use funds_manager_api::{quoters::ApiExecutionQuote, u256_try_into_u128};
use renegade_common::types::{
    chain::Chain,
    token::{Token, USDC_TICKER},
};
use tracing::warn;

use crate::{
    execution_client::{
        error::ExecutionClientError,
        venues::{
            bebop::BebopQuoteExecutionData, cowswap::CowswapQuoteExecutionData,
            lifi::LifiQuoteExecutionData, SupportedExecutionVenue,
        },
    },
    helpers::{contains_byte_subslice, get_darkpool_address, to_chain_id},
};

/// The basic information included in an execution quote,
/// agnostic of the venue that provided the quote
#[derive(Debug)]
pub struct ExecutionQuote {
    /// The token being sold
    pub sell_token: Token,
    /// The token being bought
    pub buy_token: Token,
    /// The amount of the token being sold, in atoms
    pub sell_amount: U256,
    /// The quoted amount of the token being bought, in atoms
    pub buy_amount: U256,
    /// The venue that provided the quote
    pub venue: SupportedExecutionVenue,
    /// The source of the quote
    pub source: CrossVenueQuoteSource,
    /// The chain for which the quote was generated
    pub chain: Chain,
}

impl ExecutionQuote {
    /// Whether the quote is a sell order in Renegade terms
    pub fn is_sell(&self) -> bool {
        self.buy_token == Token::from_ticker_on_chain(USDC_TICKER, self.chain)
    }

    /// Get the base token
    pub fn base_token(&self) -> Token {
        if self.is_sell() {
            self.sell_token.clone()
        } else {
            self.buy_token.clone()
        }
    }

    /// Get the base amount
    pub fn base_amount(&self) -> u128 {
        let amt_u256 = if self.is_sell() { self.sell_amount } else { self.buy_amount };
        u256_try_into_u128(amt_u256).expect("Quote amount overflows u128")
    }

    /// Get the quote token
    pub fn quote_token(&self) -> Token {
        if self.is_sell() {
            self.buy_token.clone()
        } else {
            self.sell_token.clone()
        }
    }

    /// Get the quote amount
    pub fn quote_amount(&self) -> u128 {
        let amt_u256 = if self.is_sell() { self.buy_amount } else { self.sell_amount };
        u256_try_into_u128(amt_u256).expect("Quote amount overflows u128")
    }

    /// Get the decimal-corrected quote amount, i.e. in whole units of the quote
    /// token
    pub fn quote_amount_decimal(&self) -> f64 {
        let quote_amount = self.quote_amount();
        self.quote_token().convert_to_decimal(quote_amount)
    }

    /// Get the decimal-corrected buy amount, i.e. in whole units of the buy
    /// token
    pub fn buy_amount_decimal(&self) -> f64 {
        let buy_amount = u256_try_into_u128(self.buy_amount).expect("Buy amount overflows u128");
        self.buy_token.convert_to_decimal(buy_amount)
    }

    /// Get the decimal-corrected sell amount, i.e. in whole units of the sell
    /// token
    pub fn sell_amount_decimal(&self) -> f64 {
        let sell_amount = u256_try_into_u128(self.sell_amount).expect("Sell amount overflows u128");
        self.sell_token.convert_to_decimal(sell_amount)
    }

    /// Get the price in units of USDC per base token.
    /// If a custom buy amount is provided, it is used in place of the standard
    /// buy amount.
    pub fn get_price(&self, buy_amount: Option<U256>) -> f64 {
        let buy_amount = u256_try_into_u128(buy_amount.unwrap_or(self.buy_amount))
            .expect("Buy amount overflows u128");
        let decimal_buy_amount = self.buy_token.convert_to_decimal(buy_amount);

        let decimal_sell_amount = self.sell_amount_decimal();

        let buy_per_sell = decimal_buy_amount / decimal_sell_amount;
        if self.is_sell() {
            buy_per_sell
        } else {
            1.0 / buy_per_sell
        }
    }

    /// Returns the notional volume in USDC, taking into account the actual
    /// buy amount for sell orders
    pub fn notional_volume_usdc(&self, buy_amount_actual: U256) -> f64 {
        if self.is_sell() {
            let buy_amount =
                u256_try_into_u128(buy_amount_actual).expect("Buy amount overflows u128");

            self.buy_token.convert_to_decimal(buy_amount)
        } else {
            self.sell_amount_decimal()
        }
    }
}

impl From<ExecutionQuote> for ApiExecutionQuote {
    fn from(value: ExecutionQuote) -> Self {
        let sell_token_address = value.sell_token.addr;
        let buy_token_address = value.buy_token.addr;
        let sell_amount = value.sell_amount;
        let buy_amount = value.buy_amount;
        let venue = value.venue.to_string();

        ApiExecutionQuote {
            sell_token_address,
            buy_token_address,
            sell_amount,
            buy_amount,
            venue,
            chain_id: to_chain_id(value.chain),
        }
    }
}

/// An enum wrapping the venue-specific auxiliary data needed to execute a quote
#[derive(Debug)]
pub enum QuoteExecutionData {
    /// Lifi-specific quote execution data
    Lifi(LifiQuoteExecutionData),
    /// Cowswap-specific quote execution data
    Cowswap(CowswapQuoteExecutionData),
    /// Bebop-specific quote execution data
    Bebop(BebopQuoteExecutionData),
}

impl QuoteExecutionData {
    /// "Unwraps" Lifi quote execution data, returning an error if it is not
    /// the Lifi variant
    pub fn lifi(&self) -> Result<LifiQuoteExecutionData, ExecutionClientError> {
        match &self {
            QuoteExecutionData::Lifi(data) => Ok(data.clone()),
            _ => Err(ExecutionClientError::quote_conversion("Non-Lifi quote execution data")),
        }
    }

    /// "Unwraps" Cowswap quote execution data, returning an error if it is not
    /// the Cowswap variant
    pub fn cowswap(&self) -> Result<CowswapQuoteExecutionData, ExecutionClientError> {
        match self {
            QuoteExecutionData::Cowswap(data) => Ok(data.clone()),
            _ => Err(ExecutionClientError::quote_conversion("Non-Cowswap quote execution data")),
        }
    }

    /// "Unwraps" Bebop quote execution data, returning an error if it is not
    /// the Bebop variant
    pub fn bebop(&self) -> Result<BebopQuoteExecutionData, ExecutionClientError> {
        match self {
            QuoteExecutionData::Bebop(data) => Ok(data.clone()),
            _ => Err(ExecutionClientError::quote_conversion("Non-Bebop quote execution data")),
        }
    }
}

/// An executable quote, which includes the basic quote information
/// along with any auxiliary data needed to execute the quote
#[derive(Debug)]
pub struct ExecutableQuote {
    /// The quote
    pub quote: ExecutionQuote,
    /// The venue-specific auxiliary data needed to execute the quote
    pub execution_data: QuoteExecutionData,
}

impl ExecutableQuote {
    /// Check if the executable quote is valid
    pub fn is_valid(&self) -> bool {
        // TODO: In the future, we can add more / venue-specific validation here
        !self.darkpool_address_in_calldata()
    }

    /// Check if the darkpool address is in the calldata of the quote
    fn darkpool_address_in_calldata(&self) -> bool {
        let Self { quote, execution_data } = self;
        let darkpool_address = get_darkpool_address(quote.chain);

        let calldata = match execution_data {
            QuoteExecutionData::Lifi(data) => data.data.as_ref(),
            QuoteExecutionData::Bebop(data) => data.data.as_ref(),
            // Cowswap doesn't supply calldata in quotes
            QuoteExecutionData::Cowswap(_) => return false,
        };

        let contains_darkpool_address =
            contains_byte_subslice(calldata, darkpool_address.as_slice());

        if contains_darkpool_address {
            warn!(
                "{} quote calldata contains darkpool address, suspected self-trade",
                quote.source
            );
        }

        contains_darkpool_address
    }
}

/// The different sources of quotes, across all venues
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrossVenueQuoteSource {
    /// A quote from a specific exchange via Lifi
    LifiExchange(String),
    /// A Bebop JAMv2 quote
    BebopJAMv2,
    /// A Bebop PMMv3 quote
    BebopPMMv3,
    /// A quote from Cowswap
    Cowswap,
}

impl Display for CrossVenueQuoteSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CrossVenueQuoteSource::LifiExchange(tool) => write!(f, "Lifi ({tool})"),
            CrossVenueQuoteSource::BebopJAMv2 => write!(f, "Bebop JAMv2"),
            CrossVenueQuoteSource::BebopPMMv3 => write!(f, "Bebop PMMv3"),
            CrossVenueQuoteSource::Cowswap => write!(f, "Cowswap"),
        }
    }
}
