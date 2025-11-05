//! Common database utilities

use bigdecimal::{BigDecimal, num_bigint::BigInt};
use renegade_constants::Scalar;

/// Convert a `Scalar` to a `BigDecimal`
pub fn scalar_to_bigdecimal(scalar: Scalar) -> BigDecimal {
    let bigint: BigInt = scalar.to_biguint().into();
    BigDecimal::from(bigint)
}

/// Convert a `BigDecimal` to a `Scalar`
pub fn bigdecimal_to_scalar(bigdecimal: BigDecimal) -> Scalar {
    let (bigint, scale) = bigdecimal.into_bigint_and_scale();
    debug_assert_eq!(scale, 0, "BigDecimal must have a scale of 0 for conversion to Scalar");

    Scalar::from_biguint(
        &bigint.to_biguint().expect("BigInt must be positive for conversion to Scalar"),
    )
}
