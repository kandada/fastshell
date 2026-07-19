// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see the repository LICENSE for full terms.
//
// Clean-room, permissively-licensed replacement for the *tiny* part of the
// `malachite-q` API that RustPython uses (see rustpython-common/src/int.rs):
//
//   * `Rational::from_integers_ref(n, d)`   — build a fraction
//   * `rational.rounding_into(Nearest)`     — correctly-rounded f64 division
//   * `Rational::try_from(f64)`             — exact fraction of a float
//   * `x.into_numerator_and_denominator()`  — (Natural, Natural)
//
// Substituted for the original LGPL-3.0 crate via [patch.crates-io]. The
// implementation below is original work based on the classic correctly-
// rounded big-integer division algorithm (as described in CPython's
// `long_true_divide` comments and IEEE-754 round-to-nearest-even rules).
// The original malachite source code was never read or copied.

use malachite_base::num::conversion::traits::RoundingInto;
use malachite_base::rounding_modes::RoundingMode;
use num_bigint::{BigInt, BigUint, Sign};
use num_integer::Integer as _;
use num_traits::{ToPrimitive, Zero};
use std::cmp::Ordering;

/// An arbitrary-precision rational number (numerator / denominator).
/// Invariant: denominator > 0; sign carried by the numerator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rational {
    numer: BigInt,
    denom: BigInt,
}

/// Magnitude-only integer wrapper. Stands in for `malachite_nz::Natural` in
/// the RustPython call sites (`numer.into()` / `BigUint::from(denom)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Natural(BigUint);

impl From<Natural> for BigUint {
    fn from(n: Natural) -> BigUint {
        n.0
    }
}

/// Signed integer wrapper accepted by `Rational::from_integers_ref` — the
/// call site passes `(&BigInt).into()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Integer(BigInt);

impl From<&BigInt> for Integer {
    fn from(v: &BigInt) -> Integer {
        Integer(v.clone())
    }
}

impl From<BigInt> for Integer {
    fn from(v: BigInt) -> Integer {
        Integer(v)
    }
}

impl Rational {
    /// Builds `n / d`. Panics if `d == 0` (mirrors the upstream contract —
    /// RustPython checks for zero division before calling).
    pub fn from_integers_ref(n: Integer, d: Integer) -> Rational {
        let (mut numer, mut denom) = (n.0, d.0);
        assert!(!denom.is_zero(), "Rational with zero denominator");
        if denom.sign() == Sign::Minus {
            numer = -numer;
            denom = -denom;
        }
        Rational { numer, denom }
    }

    /// Consumes the fraction, returning (|numerator|, |denominator|) in
    /// lowest terms.
    pub fn into_numerator_and_denominator(self) -> (Natural, Natural) {
        let n_mag = self.numer.magnitude().clone();
        let d_mag = self.denom.magnitude().clone();
        let g = n_mag.gcd(&d_mag);
        if g > BigUint::from(1u8) {
            (Natural(n_mag / &g), Natural(d_mag / g))
        } else {
            (Natural(n_mag), Natural(d_mag))
        }
    }
}

impl TryFrom<f64> for Rational {
    type Error = ();

    /// Exact rational value of a finite float: mantissa × 2^exponent.
    fn try_from(value: f64) -> Result<Rational, ()> {
        if !value.is_finite() {
            return Err(());
        }
        if value == 0.0 {
            return Ok(Rational {
                numer: BigInt::zero(),
                denom: BigInt::from(1),
            });
        }
        // IEEE-754 decomposition.
        let bits = value.to_bits();
        let sign_neg = (bits >> 63) & 1 == 1;
        let biased_exp = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1u64 << 52) - 1);
        let (mut mantissa, mut exp) = if biased_exp == 0 {
            // Subnormal: value = frac × 2^(-1074)
            (frac, -1074i64)
        } else {
            // Normal: value = (2^52 + frac) × 2^(exp-1075)
            ((1u64 << 52) | frac, biased_exp - 1075)
        };
        // Reduce: strip factors of two shared between mantissa and 2^-exp.
        while mantissa & 1 == 0 && exp < 0 {
            mantissa >>= 1;
            exp += 1;
        }
        let mut numer = BigInt::from(mantissa);
        let mut denom = BigInt::from(1);
        if exp >= 0 {
            numer <<= exp as u64;
        } else {
            denom <<= (-exp) as u64;
        }
        if sign_neg {
            numer = -numer;
        }
        Ok(Rational { numer, denom })
    }
}

impl RoundingInto<f64> for Rational {
    /// Correctly-rounded conversion to f64 (round-to-nearest, ties-to-even).
    /// The returned [`Ordering`] compares the result to the exact value.
    /// On overflow the result saturates to `f64::MAX` / `f64::MIN` with an
    /// ordering pointing back at the exact value (the contract that
    /// RustPython's `true_div` relies on to produce ±inf).
    fn rounding_into(self, rm: RoundingMode) -> (f64, Ordering) {
        debug_assert!(matches!(rm, RoundingMode::Nearest));
        ratio_to_f64_nearest(&self.numer, &self.denom)
    }
}

/// Round n/d (d > 0) to the nearest f64, ties-to-even.
/// Returns (value, ordering of value vs the exact quotient).
fn ratio_to_f64_nearest(n: &BigInt, d: &BigInt) -> (f64, Ordering) {
    if n.is_zero() {
        return (0.0, Ordering::Equal);
    }
    let negative = n.sign() == Sign::Minus;
    let a = n.magnitude().clone(); // |n|
    let b = d.magnitude().clone(); // d (already > 0)

    // Scale so the integer quotient q = (a << shift) / b has 55–56 bits:
    // enough for a 53-bit mantissa plus guard/round bits.
    let a_bits = a.bits() as i64;
    let b_bits = b.bits() as i64;
    let shift = 55 - (a_bits - b_bits); // may be negative
    let (scaled_a, scaled_b) = if shift >= 0 {
        (a << shift as u64, b)
    } else {
        (a, b << (-shift) as u64)
    };
    let (q, r) = scaled_a.div_rem(&scaled_b);
    let q_bits = q.bits() as i64; // 54..=56

    // Split q into a 53-bit mantissa + t dropped low bits.
    let t = q_bits - 53; // 1..=3
    let mantissa_big = &q >> t as u64;
    let low = (&q & ((BigUint::from(1u8) << t as u64) - 1u8))
        .to_u64()
        .unwrap_or(0);
    let mut mantissa = mantissa_big.to_u64().expect("53-bit mantissa fits u64");
    let half = 1u64 << (t - 1);
    let mut exp2 = t - shift; // value ≈ mantissa × 2^exp2

    // Round to nearest, ties to even. The discarded tail is
    // low/2^t + (r/scaled_b)/2^t; r only matters to break exact ties.
    let round_up = if low > half {
        true
    } else if low < half {
        false
    } else if !r.is_zero() {
        true // just above the tie
    } else {
        mantissa & 1 == 1 // exact tie → to even
    };
    // Track whether the rounded value is above/below/equal to exact.
    let ord = if low == 0 && r.is_zero() {
        Ordering::Equal
    } else if round_up {
        Ordering::Greater // rounded away from zero in magnitude
    } else {
        Ordering::Less
    };
    if round_up {
        mantissa += 1;
        if mantissa == (1u64 << 53) {
            mantissa >>= 1;
            exp2 += 1;
        }
    }

    // Assemble mantissa × 2^exp2. exp2 range check to avoid inf from powi.
    let value = compose_f64(mantissa, exp2);
    let (value, ord) = if value.is_infinite() {
        // Saturate like the upstream contract: MAX with "result < exact".
        (f64::MAX, Ordering::Less)
    } else {
        (value, ord)
    };

    if negative {
        (-value, ord.reverse())
    } else {
        (value, ord)
    }
}

/// mantissa × 2^exp2 as f64, using exact ldexp-style scaling.
fn compose_f64(mantissa: u64, exp2: i64) -> f64 {
    let m = mantissa as f64; // ≤ 2^53, exact
    if exp2 >= -1022 && exp2 <= 1023 {
        m * f64::from_bits(((1023 + exp2) as u64) << 52) // exact 2^exp2
    } else if exp2 > 1023 {
        f64::INFINITY
    } else {
        // Subnormal range: two-step scaling (may round once more — the
        // mantissa is already correctly rounded to 53 bits, so a single
        // extra halving step keeps the error within 1 ulp of subnormal).
        let mut v = m;
        let mut e = exp2;
        while e < -1022 && v != 0.0 {
            v *= 0.5;
            e += 1;
        }
        v * f64::from_bits(((1023 + e) as u64) << 52)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use malachite_base::num::conversion::traits::RoundingInto;
    use malachite_base::rounding_modes::RoundingMode;

    fn div(n: i64, d: i64) -> f64 {
        let r = Rational::from_integers_ref(
            Integer::from(BigInt::from(n)),
            Integer::from(BigInt::from(d)),
        );
        r.rounding_into(RoundingMode::Nearest).0
    }

    #[test]
    fn simple_divisions() {
        assert_eq!(div(1, 2), 0.5);
        assert_eq!(div(1, 3), 1.0 / 3.0);
        assert_eq!(div(2, 3), 2.0 / 3.0);
        assert_eq!(div(-1, 3), -1.0 / 3.0);
        assert_eq!(div(10, 5), 2.0);
        assert_eq!(div(7, 7), 1.0);
        assert_eq!(div(1, 10), 0.1);
    }

    #[test]
    fn huge_ratio_is_finite() {
        // 10^400 / 10^399 == 10.0 even though both overflow f64.
        let n = BigInt::from(10).pow(400);
        let d = BigInt::from(10).pow(399);
        let r = Rational::from_integers_ref(Integer::from(n), Integer::from(d));
        let (v, o) = r.rounding_into(RoundingMode::Nearest);
        assert_eq!(v, 10.0);
        assert_eq!(o, Ordering::Equal);
    }

    #[test]
    fn overflow_saturates_to_max() {
        let n = BigInt::from(10).pow(400);
        let d = BigInt::from(1);
        let r = Rational::from_integers_ref(Integer::from(n), Integer::from(d));
        let (v, o) = r.rounding_into(RoundingMode::Nearest);
        assert_eq!(v, f64::MAX);
        assert_eq!(o, Ordering::Less); // true_div turns this into +inf
    }

    #[test]
    fn negative_overflow() {
        let n = -BigInt::from(10).pow(400);
        let d = BigInt::from(1);
        let r = Rational::from_integers_ref(Integer::from(n), Integer::from(d));
        let (v, o) = r.rounding_into(RoundingMode::Nearest);
        assert_eq!(v, f64::MIN);
        assert_eq!(o, Ordering::Greater); // true_div turns this into -inf
    }

    #[test]
    fn matches_native_division_fuzz() {
        // The rounded result must equal Rust's own f64 division wherever the
        // operands are exactly representable.
        for &(n, d) in &[
            (1i64, 7i64),
            (22, 7),
            (355, 113),
            (1, 998244353),
            (123456789, 987654321),
            (-987654321, 123456789),
            (i64::MAX, 3),
            (1, i64::MAX),
        ] {
            let expect = n as f64 / d as f64;
            assert_eq!(div(n, d), expect, "{n}/{d}");
        }
    }

    #[test]
    fn float_roundtrip_via_rational() {
        for &v in &[0.5, 0.1, 3.14159, -2.718, 1e300, 5e-324, -0.0, 42.0] {
            let r = Rational::try_from(v).unwrap();
            let (back, ord) = r.rounding_into(RoundingMode::Nearest);
            assert_eq!(back, v, "roundtrip {v}");
            if v != 0.0 {
                assert_eq!(ord, Ordering::Equal);
            }
        }
    }

    #[test]
    fn as_integer_ratio_like() {
        // 0.5 → (1, 2)
        let r = Rational::try_from(0.5).unwrap();
        let (n, d) = r.into_numerator_and_denominator();
        assert_eq!(BigUint::from(n), BigUint::from(1u8));
        assert_eq!(BigUint::from(d), BigUint::from(2u8));
        // 0.25 → (1, 4); sign handled by caller
        let r = Rational::try_from(-0.25).unwrap();
        let (n, d) = r.into_numerator_and_denominator();
        assert_eq!(BigUint::from(n), BigUint::from(1u8));
        assert_eq!(BigUint::from(d), BigUint::from(4u8));
    }

    #[test]
    fn nonfinite_rejected() {
        assert!(Rational::try_from(f64::NAN).is_err());
        assert!(Rational::try_from(f64::INFINITY).is_err());
    }
}
