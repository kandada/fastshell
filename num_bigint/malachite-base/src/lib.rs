// Copyright (c) 2026 xiefujin <490021684@qq.com>
// Licensed under Apache-2.0, see the repository LICENSE for full terms.
//
// Clean-room, permissively-licensed replacement for the *tiny* part of the
// `malachite-base` API that RustPython uses. This crate is substituted for
// the original (LGPL-3.0-only) crate via `[patch.crates-io]` so that no LGPL
// code is ever compiled into fastshell / aacode binaries.
//
// Clean-room statement: implemented purely from the RustPython call sites
// (MIT-licensed) and public API names; the original malachite source code
// was never read or copied. See fastshell/num_bigint/README.md.

/// Rounding modes (API-name-compatible subset).
pub mod rounding_modes {
    /// How to round an inexact conversion result.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RoundingMode {
        /// Round towards zero.
        Down,
        /// Round away from zero.
        Up,
        /// Round towards negative infinity.
        Floor,
        /// Round towards positive infinity.
        Ceiling,
        /// Round to the nearest value; ties go to even.
        Nearest,
        /// The conversion must be exact.
        Exact,
    }
}

pub mod num {
    pub mod conversion {
        pub mod traits {
            use crate::rounding_modes::RoundingMode;
            use core::cmp::Ordering;

            /// Convert `self` into `T`, rounding per `rm`. Returns the value
            /// and an [`Ordering`] describing how the result compares to the
            /// exact value (`Less` = result < exact, `Equal` = exact).
            pub trait RoundingInto<T> {
                fn rounding_into(self, rm: RoundingMode) -> (T, Ordering);
            }
        }
    }

    pub mod basic {
        pub mod floats {
            /// Minimal stand-in for malachite's `PrimitiveFloat` trait.
            /// RustPython's `common/format.rs` imports it; the members below
            /// cover the used surface (extend compile-driven if needed).
            pub trait PrimitiveFloat {
                const MAX_FINITE: Self;
                const MIN_POSITIVE_SUBNORMAL: Self;

                fn is_negative_zero(self) -> bool;
                fn abs_negative_zero(self) -> Self;
            }

            impl PrimitiveFloat for f64 {
                const MAX_FINITE: f64 = f64::MAX;
                const MIN_POSITIVE_SUBNORMAL: f64 = 5e-324;

                fn is_negative_zero(self) -> bool {
                    self == 0.0 && self.is_sign_negative()
                }
                fn abs_negative_zero(self) -> f64 {
                    if self == 0.0 {
                        0.0
                    } else {
                        self
                    }
                }
            }

            impl PrimitiveFloat for f32 {
                const MAX_FINITE: f32 = f32::MAX;
                const MIN_POSITIVE_SUBNORMAL: f32 = 1e-45;

                fn is_negative_zero(self) -> bool {
                    self == 0.0 && self.is_sign_negative()
                }
                fn abs_negative_zero(self) -> f32 {
                    if self == 0.0 {
                        0.0
                    } else {
                        self
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::num::basic::floats::PrimitiveFloat;
    use super::rounding_modes::RoundingMode;

    #[test]
    fn rounding_mode_exists() {
        assert_ne!(RoundingMode::Nearest, RoundingMode::Exact);
    }

    #[test]
    fn primitive_float_negative_zero() {
        assert!((-0.0f64).is_negative_zero());
        assert!(!0.0f64.is_negative_zero());
        assert_eq!((-0.0f64).abs_negative_zero().to_bits(), 0.0f64.to_bits());
    }
}
