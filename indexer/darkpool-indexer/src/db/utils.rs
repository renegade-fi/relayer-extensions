//! Common database utilities

use bigdecimal::{BigDecimal, num_bigint::BigInt};
use renegade_circuit_types::fixed_point::FixedPoint;
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

/// Convert a `FixedPoint` to a `BigDecimal`
pub fn fixed_point_to_bigdecimal(fixed_point: FixedPoint) -> BigDecimal {
    let fp_bigint: BigInt = fixed_point.repr.to_biguint().into();

    BigDecimal::from(fp_bigint)
}

/// Convert a `BigDecimal` to a `FixedPoint`
pub fn bigdecimal_to_fixed_point(bigdecimal: BigDecimal) -> FixedPoint {
    let repr = bigdecimal_to_scalar(bigdecimal);
    FixedPoint::from_repr(repr)
}
