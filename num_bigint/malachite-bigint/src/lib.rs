// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see the repository LICENSE for full terms.
//
// Clean-room, permissively-licensed replacement for `malachite-bigint` that
// forwards to MIT/Apache-2.0 `num-bigint`. Substituted via [patch.crates-io].
// See fastshell/num_bigint/README.md.
//
// The original malachite-bigint markets itself as "a drop-in num-bigint
// replacement" — i.e. its public API is num-bigint's API. This shim simply
// goes back to the original: it re-exports num-bigint. Anything RustPython
// needs beyond this list is added compile-driven (from RustPython's
// MIT-licensed call sites only — never from malachite source).

pub use num_bigint::{BigInt, BigUint, ParseBigIntError, Sign, TryFromBigIntError};

// Conversion traits used as `malachite_bigint::ToBigInt` etc.
pub use num_bigint::{ToBigInt, ToBigUint};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_biguint_creates_signed() {
        let n = BigInt::from_biguint(Sign::Minus, BigUint::from(42u8));
        assert_eq!(n, BigInt::from(-42));
    }

    #[test]
    fn to_bigint_trait_reexported() {
        let x: BigInt = 7i64.to_bigint().unwrap();
        assert_eq!(x, BigInt::from(7));
    }
}
