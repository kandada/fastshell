#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════
# fastshell 覆盖率报告（需 `cargo install cargo-llvm-cov` + llvm-tools）
#
#   ./scripts/coverage.sh            # 摘要（每文件行覆盖率）
#   ./scripts/coverage.sh --html     # 生成 HTML 报告
# ═══════════════════════════════════════════════════════════
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "cargo-llvm-cov 未安装：cargo install cargo-llvm-cov --locked" >&2
  echo "并确保：rustup component add llvm-tools-preview" >&2
  exit 1
fi

if [ "${1:-}" = "--html" ]; then
  cargo llvm-cov --html --ignore-filename-regex '/(tests|benches|bin)/'
  echo "→ target/llvm-cov/html/index.html"
elif [ "${1:-}" = "--full" ]; then
  # Include the slow #[ignore] suites (session replay, fuzz, big flag batches).
  cargo llvm-cov --summary-only --ignore-filename-regex '/(tests|benches|bin)/' -- --include-ignored
else
  # Fast: default (non-ignored) suites only.
  cargo llvm-cov --summary-only --ignore-filename-regex '/(tests|benches|bin)/'
fi
