# fastshell/vendor — Third-Party Runtime Notes

> 中文版见 [README.zh.md](README.zh.md)

> This directory used to hold the Chaquopy CPython assets (libpython3.12 +
> stdlib, ~85 MB). **All of it was removed in 2026-07**: embedded CPython
> crashed on real devices in `unittest`/`threading` scenarios
> (pthread/bionic TLS incompatibilities — and an in-process crash takes the
> whole app down). Python support is now provided by **RustPython**
> (pure Rust, in-process safe).

## Current Python solution: RustPython

| Item | Value |
|---|---|
| Source | crates.io, **pinned exactly to `=0.5.0`** (fastshell/Cargo.toml) |
| Feature | `python-rustpython` (optional fastshell feature; enabled by aacode-rs) |
| Crates | `rustpython-vm` + `rustpython-stdlib` + `rustpython-pylib` (freeze-stdlib) |
| stdlib | CPython's pure-Python standard library **frozen into the binary** — no on-disk extraction, no PYTHONHOME |
| Engine | `fastshell/src/python/rustpython.rs` (fresh interpreter per execution, 16 MB-stack worker thread) |
| Platforms | Android (aarch64) / iOS (aarch64) / macOS / Linux — pure Rust cross-compilation |

## License composition & risk avoidance (important)

RustPython itself is **MIT** and its frozen stdlib is **PSF-2.0** (both
permissive). However, its upstream depends on the malachite family
(malachite-base / malachite-bigint / malachite-q), which is
**LGPL-3.0-only** — statically linking that into a closed-source app would
trigger LGPL's relinking obligations.

**Avoidance**: the workspace root `Cargo.toml` uses `[patch.crates-io]` to
globally replace the three malachite crates with our own clean-room
implementations (Apache-2.0, forwarding to the MIT/Apache `num-bigint`),
located at `fastshell/num_bigint/`. See the clean-room statement in
`fastshell/num_bigint/README.md`.

**The final dependency tree contains no LGPL/GPL copyleft items.** A
closed-source app only needs to ship third-party attribution texts
(Apache-2.0 / MIT / PSF-2.0 / Unicode-3.0 etc.).

## RustPython upgrade checklist (prevents LGPL re-entry — follow every step)

1. Bump the three pinned `rustpython-*` versions in `fastshell/Cargo.toml`.
2. If the new version uses more of the malachite API surface, the build will
   fail inside the shims → **fix strictly compile-driven, referring only to
   the RustPython (MIT) call sites; never open the malachite sources**
   (see the clean-room rules in num_bigint/README.md).
3. Verify the substitution still holds (must report nothing / not found):
   ```bash
   cargo tree -i malachite-nz
   ```
4. Full-tree license scan (no copyleft may appear):
   ```bash
   cargo license --features git,python-rustpython | grep -iE 'GPL|MPL'
   ```
5. Regression: `cargo test -p fastshell --features git,python-rustpython`
   (focus on the `python::rustpython` module: ast / unittest / bigint cases).
6. Android cross-compilation:
   ```bash
   cargo build --release --target aarch64-linux-android --lib -p aacode-rs
   ```

## libffi (Android only)

`libffi/aarch64-linux-android/libffi.a` is a static build of vanilla
**libffi 3.4.6** (MIT, license text at `libffi/LICENSE`) required by
rustpython-vm's ctypes support — the Android NDK ships no libffi.
It is linked via `.cargo/config.toml` rustflags and
`fastshell_c/CMakeLists.txt`. iOS does not need it.

## Historical notes

* The old CPython integration (`src/python/cpython.rs` — dlopen + extraction
  + JNI streaming callbacks, ~1100 lines) was deleted together with the
  assets; the `PythonEngine` trait abstraction remains, and desktop still
  prefers the system `python3` (SubprocessPython).
* The Android app's `libpython3.12.so` and
  `System.loadLibrary("python3.12")` were removed accordingly
  (see AACodeApp INTEGRATION.md).
