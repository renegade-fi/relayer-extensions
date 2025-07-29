//! Error types for fixed point math
use thiserror::Error;

/// Type alias for Results using FixedPointMathError
pub type FixedPointResult<T> = Result<T, FixedPointMathError>;

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
