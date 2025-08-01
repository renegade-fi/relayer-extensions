//! Venue-specific logic for getting quotes and executing swaps

use std::fmt::Display;

pub mod lifi;
pub mod quote;

/// An enum used to specify supported execution venues
pub enum SupportedExecutionVenue {
    /// The Lifi venue
    Lifi,
    /// The Cowswap venue
    Cowswap,
}

impl Display for SupportedExecutionVenue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedExecutionVenue::Lifi => write!(f, "Lifi"),
            SupportedExecutionVenue::Cowswap => write!(f, "Cowswap"),
        }
    }
}
