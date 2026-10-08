# LGPL malachite, the clean-room shims, and the vendored forks

This document explains how `fastshell` avoids the **LGPL-3.0-only** `malachite`
crates that RustPython depends on, why two vendored forks exist, and what
**downstream users of the published crates.io crate** need to know.

## Background

RustPython's crates (`rustpython-vm`, `-common`, `-compiler-core`, `-codegen`,
`-stdlib`) depend on `malachite-bigint = "0.9.1"` (a bignum implementation).
The upstream `malachite-*` crates are **LGPL-3.0-only**, which is a problem when
statically linking into a closed-source mobile app.

To keep the whole dependency tree permissively licensed, this repository ships
**clean-room, Apache-2.0** replacements for the tiny malachite surface RustPython
actually uses:

- `fastshell/num_bigint/malachite-base`
- `fastshell/num_bigint/malachite-bigint`  (forwards to `num-bigint`)
- `fastshell/num_bigint/malachite-q`

They keep the same crate names / lib names / API used by RustPython, so no
RustPython source has to change.

## How it is wired locally

The workspace root `Cargo.toml` contains:

```toml
[patch.crates-io]
malachite-base   = { path = "fastshell/num_bigint/malachite-base" }
malachite-bigint = { path = "fastshell/num_bigint/malachite-bigint" }
malachite-q      = { path = "fastshell/num_bigint/malachite-q" }
```

`[patch]` substitutes the implementation for **this workspace only**. Locally
(and for the iOS/Android app builds, which build inside this workspace) the graph
uses the Apache shims — no LGPL.

## Important: `[patch]` is NOT published

`[patch.crates-io]` is a *local* build-time override. It is **never uploaded**
and is **not inherited by downstream consumers**. Therefore:

- **Building from this repository** (path/git deps, or as the app does inside the
  workspace): you get the Apache shims automatically.
- **Depending on the published crates.io `fastshell`**: cargo cannot see this
  patch, so `malachite-*` resolves to the **real upstream LGPL** crates.

If you consume the crates.io crate and need to stay LGPL-free, add the same
`[patch.crates-io]` entry (pointing at the shim sources from this repository) to
**your own workspace root**, or build `fastshell` from this repository instead of
crates.io.

## The vendored forks (`fastshell/vendor/`)

Version drift in `pymath` was a second, independent problem:

- Every RustPython crate requires `malachite-bigint = "^0.9.1"`.
- But `pymath 0.2.0` declares `malachite-bigint = "0"` (any `0.x`).
- When malachite later published `0.10.0` / `0.11.0` / `0.12.0`, a *fresh*
  resolution of `pymath`'s `"0"` picked the newest `0.x`, which is
  semver-incompatible with `^0.9.1` → two copies of `malachite-bigint` in the
  graph → type errors at compile time (for everyone without the local patch).

To make the published crate build reproducibly for downstream users, we vendor
two **manifest-only** forks:

- `fastshell/vendor/pymath` → published as **`fastshell-pymath`**
  - `malachite-bigint` pinned from `"0"` to `"0.9"`.
  - the dev-dependency `pyo3` aligned from `0.27` to `0.22` (dev-deps are
    test-only; this avoids a `links = "python"` clash with fastshell's `pyo3`).
- `fastshell/vendor/rustpython-stdlib` → published as
  **`fastshell-rustpython-stdlib`**
  - its `pymath` dependency redirected to `fastshell-pymath`;
  - its **optional** `libsqlite3-sys` version aligned from `0.37` to `0.28`
    (matching `rusqlite`) to avoid two `links = "sqlite3"` packages. The
    `_sqlite3` module is untouched; fastshell does not enable that feature (it
    provides `sqlite3` through its own native rusqlite bridge).

**No Rust source is modified** — only `Cargo.toml` dependency wiring and two
version alignments. The forks are excluded from the workspace
(`[workspace] exclude`) so they resolve as ordinary path dependencies without
workspace-wide feature unification.

With these forks, a downstream `cargo add fastshell` resolves to a **single**
`malachite-bigint 0.9.x` and compiles. The implementation it uses is the real
upstream one (**LGPL**) unless you also apply the shim patch described above.

## Upstream tracking

The forks pin upstream `pymath 0.2.0` and `rustpython-stdlib 0.5.0`. When
RustPython publishes a release that fixes the `pymath` version range (e.g.
`^0.9`), the forks can be dropped and the upstream crates used again.

## License summary

| Component | License |
|---|---|
| `fastshell`, clean-room `malachite-*` shims, vendored forks' manifests | Apache-2.0 |
| `fastshell-pymath` (source) | PSF-2.0 |
| `fastshell-rustpython-stdlib` (source) | MIT |
| upstream real `malachite-*` (only if you consume crates.io without the patch) | LGPL-3.0-only |
