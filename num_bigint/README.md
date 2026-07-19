# fastshell/num_bigint — Clean-Room malachite Shims

> 中文版见 [README.zh.md](README.zh.md)

This directory contains **three first-party, original crates** (Apache-2.0)
that replace the identically-named LGPL-3.0-only crates RustPython depends
on, via `[patch.crates-io]` in the workspace root `Cargo.toml`:

| Directory | Replaces | Implementation |
|---|---|---|
| `malachite-bigint/` | malachite-bigint (LGPL-3.0) | forwards to **num-bigint** (MIT/Apache-2.0) |
| `malachite-q/` | malachite-q (LGPL-3.0) | original `Rational` on top of num-bigint (incl. correctly-rounded big-int → f64 division) |
| `malachite-base/` | malachite-base (LGPL-3.0) | original micro-traits: `RoundingMode` / `RoundingInto` / `PrimitiveFloat` |

## ⚠️ Package name vs. license

These three crates carry the **same package names as the original LGPL
crates** (the `[patch.crates-io]` mechanism requires matching name+version),
but **the code is an original work of this repository, licensed Apache-2.0**,
with no code relationship whatsoever to Mikhail Hogrefe's malachite project.
If a license-audit tool flags "malachite-*" by name, the copyright headers in
this directory's sources and this README are authoritative.

## Clean-Room Statement

The code in this directory was produced under the following rules; this
statement itself serves as compliance evidence:

1. **The malachite family sources (LGPL-3.0-only) were never read, copied,
   or consulted.**
2. The two legitimate sources for API shape:
   * **the public API of num-bigint / num-rational** (MIT/Apache-2.0) —
     malachite-bigint's own description is "a drop-in num-bigint
     replacement", i.e. that API originates from num-bigint in the first
     place;
   * **RustPython's call sites** (MIT) — which functions are needed and
     with which signatures is determined from the RustPython sources
     (`rustpython-common/src/int.rs` etc.) and **compiler errors**
     (compile-driven development).
3. The algorithms are original: `malachite-q`'s `rounding_into`
   (correctly-rounded big-integer ratio to f64) is implemented
   independently from the IEEE-754 round-to-nearest-ties-to-even rules and
   the publicly documented approach of CPython's `long_true_divide`,
   with unit-test coverage (huge ratios, overflow saturation, round-trips,
   agreement with native division).

## Rules for adding APIs (future maintainers, read this)

If a RustPython upgrade makes the compiler report a missing API in a shim:

1. Open **the RustPython source file at the error site** (MIT) to confirm
   the call pattern and expected semantics;
2. If num-bigint / num-rational has a matching implementation → forward to it;
3. If not → implement originally from the call-site semantics + public
   standards (IEEE-754 / CPython documentation);
4. **Under no circumstances open the malachite sources "just to check"** —
   a single exposure taints the clean-room property;
5. Add a unit test, and state the API-source rationale in the PR/commit
   message.

## Verification commands

```bash
# Substitution effective: malachite-nz (the real malachite core) must be
# absent from the dependency graph
cargo tree -i malachite-nz          # expect: error / nothing depends on it

# Unit tests of the three shims
cargo test -p malachite-base -p malachite-bigint -p malachite-q

# End-to-end (RustPython running on the shims)
cargo test -p fastshell --features python-rustpython --lib rustpython
```
