//! Error types for fixed point math
use alloy_primitives::U256;
use thiserror::Error;

/// Type alias for Results using FixedPointMathError
pub type FixedPointResult = Result<U256, FixedPointMathError>;

/// The error type for fixed point math
#[derive(Error, Debug)]
pub enum FixedPointMathError {
    /// Division by zero
    #[error("Division by zero")]
    DivisionByZero,
    /// Overflow
    #[error("Overflow")]
    Overflow,
}
