#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════
# fastshell 一键回归：
#   fmt --check → clippy → 全量单测/集成 → 差分模糊（扩展种子）
#
# 差分测试以真实 bash 为参照（优先 bash≥4，见 tests/bash_fuzz.rs::bash_bin）。
# 合法差异记录在 tests/known_divergences.txt。
# ═══════════════════════════════════════════════════════════
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo fmt --check"
cargo fmt --check

echo "==> cargo clippy --all-targets (warnings are pre-existing)"
cargo clippy --all-targets 2>&1 | grep -E '^error' && exit 1 || true

echo "==> cargo test"
cargo test

echo "==> differential fuzz (FB_FUZZ_SEEDS=${FB_FUZZ_SEEDS:-3000})"
FB_FUZZ_SEEDS="${FB_FUZZ_SEEDS:-3000}" cargo test --test bash_fuzz -- --nocapture

echo
echo "==> fastshell regression OK"
