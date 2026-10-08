#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════
# fastshell 重型测试入口。
#
# 这些用例平时都带 #[ignore]，不会进入 `cargo test`（默认很快）。
# 本脚本默认只跑「快且专项」的 jq/awk 引擎套件。
#
# 用法：
#   bash scripts/full-test.sh                 # 仅 jq/awk（快，秒级）
#   bash scripts/full-test.sh --test <name>   # 指定某个套件
#   bash scripts/full-test.sh --all           # 跑全部 #[ignore]（很慢）
#
# ⚠️ --all 会打开下列耗时/联网套件（默认一律不跑）：
#   stress_test / bash_fuzz* / curl_flags(联网) / session_replay /
#   sys_smoke / sys_extra_smoke / device_mock / device_callback /
#   device_bridge_test / ffi_abi / big_flags* / cmd_flags* /
#   builtin_matrix / posix_sh / bash_diff_test
# ═══════════════════════════════════════════════════════════
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" = "--all" ]; then
    shift || true
    echo "⚠️  运行全部 #[ignore]（含耗时/联网套件，可能数分钟）"
    cargo test "$@" -- --ignored --nocapture
    exit 0
fi

if [ "$#" -gt 0 ]; then
    echo "==> cargo test $* -- --ignored --nocapture"
    cargo test "$@" -- --ignored --nocapture
    exit 0
fi

echo "==> jq / awk extended engine (fast)"
cargo test --test jq_awk_full -- --ignored --nocapture
echo
echo "==> fastshell dedicated tests OK (use --all for the slow suites)"
