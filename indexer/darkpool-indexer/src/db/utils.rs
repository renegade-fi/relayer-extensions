//! Common database utilities

use bigdecimal::{BigDecimal, num_bigint::BigInt};
use renegade_constants::Scalar;

/// Convert a `Scalar` to a `BigDecimal`
pub fn scalar_to_bigdecimal(scalar: Scalar) -> BigDecimal {
    let bigint: BigInt = scalar.to_biguint().into();
    BigDecimal::from(bigint)
}
